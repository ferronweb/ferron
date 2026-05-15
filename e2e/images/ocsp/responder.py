#!/usr/bin/env python3
import os
import sys
from datetime import datetime, timedelta, timezone

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
from flask import Flask, Response

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


def make_ocsp_response():
    # Build a simple OCSP response for the generated server cert (signed by CA key)
    server_cert = x509.load_pem_x509_certificate(open(SERVER_NOCHAIN_CRT, "rb").read())
    ca_cert = x509.load_pem_x509_certificate(open(CA_CRT, "rb").read())
    ca_key = serialization.load_pem_private_key(
        open(CA_KEY, "rb").read(), password=None
    )

    # The only supported CA key types are:
    # - `Ed25519PrivateKey`
    # - `Ed448PrivateKey`
    # - `RSAPrivateKey`
    # - `DSAPrivateKey`
    # - `EllipticCurvePrivateKey`
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
        raise ValueError("ca_key must be an RSA private key")

    builder = OCSPResponseBuilder()
    this_update = datetime.now(timezone.utc)
    next_update = this_update + timedelta(hours=1)

    # add_response args may be positional or named; use keyword names where available
    builder = builder.add_response(
        cert=server_cert,
        issuer=ca_cert,
        algorithm=hashes.SHA256(),
        cert_status=OCSPCertStatus.GOOD,
        this_update=this_update,
        next_update=next_update,
        revocation_time=None,
        revocation_reason=None,
    ).responder_id(OCSPResponderEncoding.HASH, ca_cert)

    # Sign with CA key and return DER bytes
    ocsp_resp = builder.sign(ca_key, hashes.SHA256())
    return ocsp_resp.public_bytes(serialization.Encoding.DER)


app = Flask(__name__)


@app.route("/ready", methods=["GET"])
def ready():
    return "OK\n", 200


@app.route("/", methods=["POST"])
def ocsp():
    try:
        # For simplicity respond with OCSP for the server cert regardless of request contents
        resp = make_ocsp_response()
        return Response(resp, content_type="application/ocsp-response")
    except Exception as e:
        print("ocsp error:", e)
        return ("", 500)


if __name__ == "__main__":
    os.makedirs(CERT_DIR, exist_ok=True)
    generate()
    # Run Flask server
    app.run(host="0.0.0.0", port=5000)
