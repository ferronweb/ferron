use std::io::Write;

use crate::common;

/// Host configuration smoke test: requests with the correct Host header
/// should reach the right virtual host and serve the file.
#[tokio::test]
async fn test_host_configuration() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    let basic_content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Maecenas id dignissim leo, ac imperdiet tellus. Orci varius natoque penatibus et magnis dis parturient montes, nascetur ridiculus mus. Maecenas id erat finibus, auctor odio eu, efficitur libero. Aenean aliquet vehicula nisi ac tincidunt. Donec non vulputate dolor. Sed faucibus pulvinar augue eget viverra. Donec ornare lacus non mi mollis lacinia. Nulla suscipit vestibulum maximus. Nulla sit amet ex quis purus imperdiet vestibulum eget quis ex. Nullam accumsan nibh massa, vitae rhoncus sapien ultricies vel.\n";

    config_file
        .as_file_mut()
        .write_all(
            r#"
      snippet WORDPRESS_SCAN {
        status 403
      }

      match WORDPRESS_SCAN {
        request.uri ~ "(?i)^/wp-(?:login\.php|admin/?)(?:$|[?#])"
      }

      match SOMESCANNER_SCAN {
        request.header.user-agent ~ "^somescanner(/|$)"
      }

      aunrel:80 {
        status 403
      }

      ferron:80 {
        root {{env.FERRON_ROOT}}

        location /phpmyadmin {
          status 403
        }

        if WORDPRESS_SCAN {
          use WORDPRESS_SCAN
        }

        if SOMESCANNER_SCAN {
          status 403
        }

        handle_error 404 {
          status 302 {
            regex "^/(?!basic\.txt(?:$|[?#]))"
            location "/basic.txt"
          }
        }
      }
  "#
            .as_bytes(),
        )
        .unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(80)
        .await
        .unwrap();
    let client = reqwest::Client::new();

    common::write_file(
        webroot_dir.path().join("basic.txt"),
        basic_content.as_bytes(),
    )
    .unwrap();

    // Test 1: Host configuration smoke test
    let response = client
        .get(format!("http://localhost:{}/basic.txt", port))
        .header("Host", "ferron")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), basic_content);

    // Test 2: Access denial with location (exact URL)
    let response = client
        .get(format!("http://localhost:{}/phpmyadmin", port))
        .header("Host", "ferron")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 3: Access denial with location (subdirectory)
    let response = client
        .get(format!("http://localhost:{}/phpmyadmin/index.php", port))
        .header("Host", "ferron")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 4: Access denial with regex conditional and snippet
    let response = client
        .get(format!("http://localhost:{}/wp-login.php", port))
        .header("Host", "ferron")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 5: Access denial with Rego-based conditional
    let response = client
        .get(format!("http://localhost:{}/", port))
        .header("Host", "ferron")
        .header("User-Agent", "somescanner/0.0.0")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    container.stop().await.unwrap();
}
