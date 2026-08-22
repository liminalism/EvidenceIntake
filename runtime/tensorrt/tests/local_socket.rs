#![allow(missing_docs)]

use std::time::Duration;

use evidence_trt::{Broker, Client, Request, ResultBody};

#[test]
fn client_and_broker_negotiate_over_the_namespaced_local_transport() {
    let temporary = tempfile::tempdir().unwrap();
    let endpoint = format!("evidence-trt-test-{}", std::process::id());
    let broker = Broker::discover(temporary.path(), 1024, "test-gpu", "10.0", None).unwrap();
    let serving_endpoint = endpoint.clone();
    std::thread::spawn(move || {
        let _ = evidence_trt::server::serve(&serving_endpoint, broker);
    });

    let mut client = (0..100)
        .find_map(|_| {
            let connected = Client::connect(&endpoint).ok();
            if connected.is_none() {
                std::thread::sleep(Duration::from_millis(5));
            }
            connected
        })
        .expect("broker endpoint became available");
    let ResultBody::Health(health) = client.request(Request::Health, &[]).unwrap() else {
        panic!("health request returned another result");
    };
    assert!(health.ready);
    assert_eq!(health.gpu, "test-gpu");
}
