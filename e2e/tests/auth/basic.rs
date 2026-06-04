use std::io::Write;

use testcontainers::core::ContainerPort;


#[tokio::test]
async fn test_http_auth_success() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = crate::common::create_temp_dir();
    let mut config_file = crate::common::create_temp_file();

    let password_hash = password_auth::generate_hash("test");

    config_file
        .as_file_mut()
        .write_all(
            format!(
                r#"
*:80 {{
  basic_auth {{
    realm "HTTP authentication test"
    users {{
      test "{password_hash}"
    }}
  }}
  root "/var/www/ferron"
}}
"#
            )
            .as_bytes(),
        )
        .unwrap();

    crate::common::write_file(
        webroot_dir.path().join("test.txt").to_path_buf(),
        b"test content",
    )
    .unwrap();

    let container = crate::common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .basic_auth("test", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "test content");

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_http_auth_failure() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = crate::common::create_temp_dir();
    let mut config_file = crate::common::create_temp_file();

    let password_hash = "$argon2id$v=19$m=65536,t=3,p=1$c2VjcmV0c2FsdDEyMzQ1Njc4$R7dF5Q8QYJZQYJZQYJZQYJZQYJZQYJZQYJZQYJZQYJQ";

    config_file
        .as_file_mut()
        .write_all(
            format!(
                r#"
*:80 {{
  basic_auth {{
    realm "HTTP authentication test"
    users {{
      test "{password_hash}"
    }}
  }}
  root "/var/www/ferron"
}}
"#
            )
            .as_bytes(),
        )
        .unwrap();

    crate::common::write_file(
        webroot_dir.path().join("test.txt").to_path_buf(),
        b"test content",
    )
    .unwrap();

    let container = crate::common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .basic_auth("test", Some("wrong_password"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_http_auth_too_many_attempts() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = crate::common::create_temp_dir();
    let mut config_file = crate::common::create_temp_file();

    let password_hash = "$argon2id$v=19$m=65536,t=3,p=1$c2VjcmV0c2FsdDEyMzQ1Njc4$R7dF5Q8QYJZQYJZQYJZQYJZQYJZQYJZQYJZQYJZQYJQ";

    config_file
        .as_file_mut()
        .write_all(
            format!(
                r#"
*:80 {{
  basic_auth {{
    realm "HTTP authentication test"
    users {{
      test "{password_hash}"
    }}

    brute_force_protection {{
        enabled true
        max_attempts 3
        lockout_duration "15m"
        window "5m"
    }}
  }}
  root "/var/www/ferron"
}}
"#
            )
            .as_bytes(),
        )
        .unwrap();

    crate::common::write_file(
        webroot_dir.path().join("test.txt").to_path_buf(),
        b"test content",
    )
    .unwrap();

    let container = crate::common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .basic_auth("test", Some("wrong_password"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2 more attempts should fail before lockout
    client
        .get(format!("http://localhost:{}/test.txt", port))
        .basic_auth("test", Some("wrong_password"))
        .send()
        .await
        .unwrap();
    client
        .get(format!("http://localhost:{}/test.txt", port))
        .basic_auth("test", Some("wrong_password"))
        .send()
        .await
        .unwrap();

    // 4th attempt should fail before lockout
    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .basic_auth("test", Some("wrong_password"))
        .send()
        .await
        .unwrap();
    assert_ne!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

    container.stop().await.unwrap();
}
