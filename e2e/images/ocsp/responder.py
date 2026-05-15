#!/usr/bin/env python3
import hashlib
import os
import sys
from datetime import datetime, timedelta, timezone

from asn1crypto import core as asn1core
from asn1crypto import ocsp as asn1ocsp
from asn1crypto import pem as asn1pem
from asn1crypto import x509 as asn1x509

# Cryptography imports
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import dsa, ec, ed448, ed25519, rsa
from cryptography.x509.ocsp import (
    OCSPCertStatus,
    OCSPResponderEncoding,
    OCSPResponseBuilder,
)
from cryptography.x509.oid import AuthorityInformationAccessOID, NameOID
from flask import Flask, Response, request

CERT_DIR = sys.argv[1] if len(sys.argv) > 1 else "/certs"
SERVER_CRT = os.path.join(CERT_DIR, "server.crt")
SERVER_KEY = os.path.join(CERT_DIR, "server.key")
SERVER_NOCHAIN_CRT = os.path.join(CERT_DIR, "server_nochain.crt")
CA_CRT = os.path.join(CERT_DIR, "ca.crt")
CA_KEY = os.path.join(CERT_DIR, "ca.key")


def write_pem(path, data):
    with open(path, "wb") as f:
        f.write(data)


def generate():
    # If certs already present, skip generation
    if (
        os.path.exists(SERVER_CRT)
        and os.path.exists(SERVER_KEY)
        and os.path.exists(CA_CRT)
        and os.path.exists(CA_KEY)
    ):
        print("certs already present")
        return

    # Generate CA key & cert
    ca_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    ca_name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Test CA")])
    ca_cert = (
        x509.CertificateBuilder()
        .subject_name(ca_name)
        .issuer_name(ca_name)
        .public_key(ca_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(datetime.now(timezone.utc) - timedelta(days=1))
        .not_valid_after(datetime.now(timezone.utc) + timedelta(days=3650))
        .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
        .sign(ca_key, hashes.SHA256())
    )

    write_pem(
        CA_KEY,
        ca_key.private_bytes(
            encoding=serialization.Encoding.PEM,
            format=serialization.PrivateFormat.TraditionalOpenSSL,
            encryption_algorithm=serialization.NoEncryption(),
        ),
    )
    write_pem(CA_CRT, ca_cert.public_bytes(serialization.Encoding.PEM))

    # Generate server key & cert with SAN=localhost and AIA OCSP pointing to ocsp container hostname
    server_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    server_name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "localhost")])
    san = x509.SubjectAlternativeName([x509.DNSName("localhost")])
    access_desc = x509.AccessDescription(
        AuthorityInformationAccessOID.OCSP,
        x509.UniformResourceIdentifier("http://ocsp:5000/"),
    )
    aia = x509.AuthorityInformationAccess([access_desc])

    server_cert = (
        x509.CertificateBuilder()
        .subject_name(server_name)
        .issuer_name(ca_cert.subject)
        .public_key(server_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(datetime.now(timezone.utc) - timedelta(days=1))
        .not_valid_after(datetime.now(timezone.utc) + timedelta(days=365))
        .add_extension(san, critical=False)
        .add_extension(aia, critical=False)
        .sign(ca_key, hashes.SHA256())
    )

    write_pem(
        SERVER_KEY,
        server_key.private_bytes(
            encoding=serialization.Encoding.PEM,
            format=serialization.PrivateFormat.TraditionalOpenSSL,
            encryption_algorithm=serialization.NoEncryption(),
        ),
    )
    write_pem(
        SERVER_NOCHAIN_CRT,
        server_cert.public_bytes(serialization.Encoding.PEM),
    )
    # Write server cert along with the CA certificate as certificate chain
    write_pem(
        SERVER_CRT,
        server_cert.public_bytes(serialization.Encoding.PEM)
        + ca_cert.public_bytes(serialization.Encoding.PEM),
    )


def make_ocsp_response_for(cert, issuer, algo, status=OCSPCertStatus.GOOD):
    # ca_cert = issuer
    ca_key = serialization.load_pem_private_key(
        open(CA_KEY, "rb").read(), password=None
    )

    if not isinstance(
        ca_key,
        (
            ed25519.Ed25519PrivateKey,
            ed448.Ed448PrivateKey,
            rsa.RSAPrivateKey,
            dsa.DSAPrivateKey,
            ec.EllipticCurvePrivateKey,
        ),
    ):
        raise ValueError("ca_key must be a supported private key type")

    builder = OCSPResponseBuilder()
    this_update = datetime.now(timezone.utc)
    next_update = this_update + timedelta(hours=1)

    builder = builder.add_response(
        cert=cert,
        issuer=issuer,
        algorithm=algo,
        cert_status=status,
        this_update=this_update,
        next_update=next_update,
        revocation_time=None,
        revocation_reason=None,
    ).responder_id(OCSPResponderEncoding.HASH, issuer)

    ocsp_resp = builder.sign(ca_key, hashes.SHA256())
    return ocsp_resp.public_bytes(serialization.Encoding.DER)


def parse_ocsp_request(data):
    # data is raw DER
    try:
        req = asn1ocsp.OCSPRequest.load(data)
    except Exception:
        # Maybe PEM-wrapped
        if asn1pem.detect(data):
            _, _, der = asn1pem.unarmor(data)
            req = asn1ocsp.OCSPRequest.load(der)
        else:
            raise

    req_list = req["tbs_request"]["request_list"]
    if len(req_list) == 0:
        raise ValueError("OCSP request contains no Request entries")
    req_cert = req_list[0]["req_cert"]
    hash_oid = req_cert["hash_algorithm"]["algorithm"].dotted
    issuer_name_hash = bytes(req_cert["issuer_name_hash"])
    issuer_key_hash = bytes(req_cert["issuer_key_hash"])
    serial_number = int(req_cert["serial_number"].native)
    return hash_oid, issuer_name_hash, issuer_key_hash, serial_number


app = Flask(__name__)


@app.route("/ready", methods=["GET"])
def ready():
    return "OK\n", 200


@app.route("/", methods=["POST"])
def ocsp():
    try:
        req_der = request.get_data()
        # Parse OCSP request and validate CertID
        try:
            hash_oid, issuer_name_hash, issuer_key_hash, serial = parse_ocsp_request(
                req_der
            )
        except Exception as e:
            print("Failed to parse OCSP request:", e)
            return ("", 400)

        # Load CA cert to compute expected hashes
        ca_pem = open(CA_CRT, "rb").read()
        if asn1pem.detect(ca_pem):
            _, _, ca_der = asn1pem.unarmor(ca_pem)
        else:
            ca_der = ca_pem
        ca_asn = asn1x509.Certificate.load(ca_der)
        issuer_name_der = ca_asn["tbs_certificate"]["subject"].dump()
        # subject_public_key_info.public_key is a BitString
        pubkey_bitstring = (
            ca_asn["tbs_certificate"]["subject_public_key_info"]["public_key"]
            .cast(asn1core.OctetBitString)
            .native
        )

        # Reject request if public key is not a valid bitstring
        if not isinstance(pubkey_bitstring, bytes):
            print("Invalid public key bit string")
            return ("", 500)

        # Determine hash algorithm
        if hash_oid in ("2.16.840.1.101.3.4.2.1",):
            # sha256
            name_hash = hashlib.sha256(issuer_name_der).digest()
            key_hash = hashlib.sha256(pubkey_bitstring).digest()
            algo = hashes.SHA256()
        elif hash_oid in ("1.3.14.3.2.26",):
            # sha1
            name_hash = hashlib.sha1(issuer_name_der).digest()
            key_hash = hashlib.sha1(pubkey_bitstring).digest()
            algo = hashes.SHA1()
        else:
            # default to sha256
            name_hash = hashlib.sha256(issuer_name_der).digest()
            key_hash = hashlib.sha256(pubkey_bitstring).digest()
            algo = hashes.SHA256()

        # Ensure issuer binding matches
        if name_hash != issuer_name_hash or key_hash != issuer_key_hash:
            print("Issuer binding does not match CA cert")
            return ("", 400)

        # Load server cert to compare serial
        server_cert = x509.load_pem_x509_certificate(
            open(SERVER_NOCHAIN_CRT, "rb").read()
        )

        if serial != server_cert.serial_number:
            # Return a successful OCSP response but with UNKNOWN status for the requested serial
            ca_cert_obj = x509.load_pem_x509_certificate(open(CA_CRT, "rb").read())
            resp = make_ocsp_response_for(
                server_cert,
                ca_cert_obj,
                # pubkey_bitstring,
                algo,
                status=OCSPCertStatus.UNKNOWN,
            )
            return Response(resp, content_type="application/ocsp-response")

        # Serial matches; return GOOD
        ca_cert_obj = x509.load_pem_x509_certificate(open(CA_CRT, "rb").read())
        resp = make_ocsp_response_for(
            server_cert,
            ca_cert_obj,
            # pubkey_bitstring,
            algo,
            status=OCSPCertStatus.GOOD,
        )
        return Response(resp, content_type="application/ocsp-response")
    except Exception as e:
        print("ocsp error:", e)
        return ("", 500)


if __name__ == "__main__":
    os.makedirs(CERT_DIR, exist_ok=True)
    generate()
    # Run Flask server
    app.run(host="0.0.0.0", port=5000)
