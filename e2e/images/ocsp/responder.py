#!/usr/bin/env python3
import hashlib
import os
import sys
from datetime import datetime, timedelta, timezone

# Cryptography imports
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import dsa, ec, ed448, ed25519, rsa
from cryptography.x509 import ocsp as x509_ocsp
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
if os.environ.get("FERRON_E2E_OCSP_SEPARATE_SIGNER") == "1":
    OCSP_CRT = os.path.join(CERT_DIR, "ocsp.crt")
    OCSP_KEY = os.path.join(CERT_DIR, "ocsp.key")
else:
    OCSP_CRT = CA_CRT
    OCSP_KEY = CA_KEY


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

    if OCSP_CRT != CA_CRT:
        # Generate OCSP responder key and certificate
        ocsp_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        eku = x509.ExtendedKeyUsage([x509.OID_OCSP_SIGNING])

        ocsp_cert_name = x509.Name(
            [x509.NameAttribute(NameOID.COMMON_NAME, "Test CA OCSP Signer")]
        )
        ocsp_cert = (
            x509.CertificateBuilder()
            .subject_name(ocsp_cert_name)
            .issuer_name(ca_cert.subject)
            .public_key(ocsp_key.public_key())
            .serial_number(x509.random_serial_number())
            .not_valid_before(datetime.now(timezone.utc) - timedelta(days=1))
            .not_valid_after(datetime.now(timezone.utc) + timedelta(days=365))
            .add_extension(eku, critical=False)
            .sign(ca_key, hashes.SHA256())
        )

        write_pem(
            OCSP_CRT,
            ocsp_cert.public_bytes(serialization.Encoding.PEM),
        )
        write_pem(
            OCSP_KEY,
            ocsp_key.private_bytes(
                encoding=serialization.Encoding.PEM,
                format=serialization.PrivateFormat.TraditionalOpenSSL,
                encryption_algorithm=serialization.NoEncryption(),
            ),
        )


def make_ocsp_response_for(cert, issuer, algo, status=OCSPCertStatus.GOOD):
    ocsp_cert = x509.load_pem_x509_certificate(open(OCSP_CRT, "rb").read())
    ocsp_key = serialization.load_pem_private_key(
        open(OCSP_KEY, "rb").read(), password=None
    )

    if not isinstance(
        ocsp_key,
        (
            ed25519.Ed25519PrivateKey,
            ed448.Ed448PrivateKey,
            rsa.RSAPrivateKey,
            dsa.DSAPrivateKey,
            ec.EllipticCurvePrivateKey,
        ),
    ):
        raise ValueError("OCSP responder key must be a supported private key type")

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
    )

    # "Forge" the signature if an environment variable is set to 1
    if os.environ.get("FERRON_E2E_OCSP_FORGE_SIGNATURE") == "1":
        # Simulate forged OCSP response
        forged_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        forged_cert = (
            x509.CertificateBuilder()
            .subject_name(issuer.subject)
            .issuer_name(issuer.subject)
            .public_key(forged_key.public_key())
            .serial_number(x509.random_serial_number())
            .not_valid_before(datetime.now(timezone.utc) - timedelta(days=1))
            .not_valid_after(datetime.now(timezone.utc) + timedelta(days=365))
            .sign(forged_key, hashes.SHA256())
        )
        ocsp_resp = builder.responder_id(OCSPResponderEncoding.HASH, forged_cert).sign(
            forged_key, hashes.SHA256()
        )
    else:
        if CA_CRT != OCSP_CRT:
            ocsp_resp = (
                builder.certificates([ocsp_cert])
                .responder_id(OCSPResponderEncoding.HASH, ocsp_cert)
                .sign(ocsp_key, hashes.SHA256())
            )
        else:
            ocsp_resp = builder.responder_id(
                OCSPResponderEncoding.HASH, ocsp_cert
            ).sign(ocsp_key, hashes.SHA256())

    return ocsp_resp.public_bytes(serialization.Encoding.DER)


def parse_ocsp_request(data):
    # data is raw DER
    req = x509_ocsp.load_der_ocsp_request(data)

    hash = req.hash_algorithm.name
    issuer_name_hash = req.issuer_name_hash
    issuer_key_hash = req.issuer_key_hash
    serial_number = req.serial_number
    return hash, issuer_name_hash, issuer_key_hash, serial_number


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
            hash, issuer_name_hash, issuer_key_hash, serial = parse_ocsp_request(
                req_der
            )
        except Exception as e:
            print("Failed to parse OCSP request:", e)
            return ("", 400)

        # Load CA cert to compute expected hashes
        ca_pem = open(CA_CRT, "rb").read()
        ca_asn = x509.load_pem_x509_certificate(ca_pem)
        issuer_name_der = ca_asn.subject.public_bytes()
        # subject_public_key_info.public_key is a BitString
        pubkey_bitstring = ca_asn.public_key().public_bytes(
            encoding=serialization.Encoding.DER, format=serialization.PublicFormat.PKCS1
        )

        # Determine hash algorithm
        if hash == "sha256":
            # sha256
            name_hash = hashlib.sha256(issuer_name_der).digest()
            key_hash = hashlib.sha256(pubkey_bitstring).digest()
            algo = hashes.SHA256()
        elif hash == "sha1":
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
            resp = make_ocsp_response_for(
                server_cert,
                ca_asn,
                algo,
                status=OCSPCertStatus.UNKNOWN,
            )
            return Response(resp, content_type="application/ocsp-response")

        # Serial matches; return GOOD
        resp = make_ocsp_response_for(
            server_cert,
            ca_asn,
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
