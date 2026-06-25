use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_ferron_container() -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network("bridge")
        .with_cmd(vec!["/bin/sh", "-c", "ferron daemon -c /etc/ferron.conf --pid-file /tmp/ferron.pid; sleep 0.1; wait $(cat /tmp/ferron.pid)"])
        .start()
        .await
}

#[tokio::test]
async fn test_unix() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    let container = create_ferron_container().await.unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // Test before reload
    let response = client
        .get(format!("http://localhost:{}", port))
        .send()
        .await
        .unwrap();

    // Default Ferron configuration from the test container has
    // a "Ferron is installed successfully!" page.
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    container.stop().await.unwrap();
}
