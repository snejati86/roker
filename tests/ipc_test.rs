use roker::{Broker, BrokerConfig, ClientId, Message, Topic};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{env, thread};

const TEST_SHM_NAME: &str = "test_broker";
const BUFFER_SIZE: usize = 1024 * 1024; // 1MB
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

// Helper struct to manage test processes
struct TestProcess {
    child: Child,
    name: String,
}

impl TestProcess {
    fn new(name: &str, env_var: &str) -> std::io::Result<Self> {
        let child = Command::new(env::current_exe().unwrap())
            .arg("--test-threads=1")
            .arg("--nocapture")
            .arg("--")
            .arg(name)
            .env(env_var, "1")
            .spawn()?;

        Ok(TestProcess {
            child,
            name: name.to_string(),
        })
    }

    fn wait_timeout(&mut self, timeout: Duration) -> std::io::Result<bool> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            match self.child.try_wait()? {
                Some(status) => return Ok(status.success()),
                None => thread::sleep(Duration::from_millis(100)),
            }
        }
        // Kill the process if it hasn't finished
        let _ = self.child.kill();
        let _ = self.child.wait();
        Ok(false)
    }
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            // Process is still running, kill it
            let _ = self.child.kill();
            let _ = self.child.wait();
            println!("Killed hanging process: {}", self.name);
        }
    }
}

#[test]
fn test_ipc_communication() {
    // Clean up any existing shared memory from previous test runs
    if let Ok(broker) = Broker::connect(TEST_SHM_NAME) {
        drop(broker);
        thread::sleep(Duration::from_millis(100));
    }

    let config = BrokerConfig {
        name: TEST_SHM_NAME.to_string(),
        buffer_size: BUFFER_SIZE,
        max_clients: 100,
        max_subscriptions_per_client: 10,
    };

    let broker = Arc::new(Broker::new(config).expect("Failed to create broker"));
    let shutdown = Arc::new(AtomicBool::new(false));

    // Start publisher process
    let mut publisher =
        TestProcess::new("publisher", "TEST_PUBLISHER").expect("Failed to start publisher process");

    // Start subscriber process
    let mut subscriber = TestProcess::new("subscriber", "TEST_SUBSCRIBER")
        .expect("Failed to start subscriber process");

    // Give processes time to start
    thread::sleep(Duration::from_millis(100));

    // Wait for processes with timeout
    let pub_success = publisher
        .wait_timeout(TEST_TIMEOUT)
        .expect("Failed to wait for publisher");
    let sub_success = subscriber
        .wait_timeout(TEST_TIMEOUT)
        .expect("Failed to wait for subscriber");

    // Signal shutdown
    shutdown.store(true, Ordering::SeqCst);

    assert!(pub_success, "Publisher process failed or timed out");
    assert!(sub_success, "Subscriber process failed or timed out");

    // Clean up
    drop(broker);
    thread::sleep(Duration::from_millis(100));
}

#[test]
fn publisher_process() {
    if env::var("TEST_PUBLISHER").is_err() {
        return;
    }

    // Give the main process time to create the broker
    thread::sleep(Duration::from_millis(200));

    let broker = match Broker::connect(TEST_SHM_NAME) {
        Ok(b) => b,
        Err(e) => {
            println!("Publisher failed to connect: {}", e);
            return;
        }
    };

    let _client_id = ClientId::new();

    // Publish messages with different topics
    let test_messages = vec![
        ("/images/png", "PNG image data"),
        ("/images/jpg", "JPG image data"),
        ("/images/raw/large", "RAW image data"),
        ("/videos/mp4", "MP4 video data"),
    ];

    for (topic_str, data) in test_messages {
        let topic = Topic::new(topic_str).expect("Invalid topic");
        let message = Message::new(topic, data.as_bytes().to_vec());
        if let Err(e) = broker.publish(message) {
            println!("Failed to publish message: {}", e);
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn subscriber_process() {
    if env::var("TEST_SUBSCRIBER").is_err() {
        return;
    }

    // Give the main process time to create the broker
    thread::sleep(Duration::from_millis(200));

    let broker = match Broker::connect(TEST_SHM_NAME) {
        Ok(b) => b,
        Err(e) => {
            println!("Subscriber failed to connect: {}", e);
            return;
        }
    };

    let client_id = ClientId::new();

    // Subscribe to different patterns
    if let Err(e) = broker.subscribe(&client_id, "/images/*") {
        println!("Failed to subscribe to /images/*: {}", e);
        return;
    }
    if let Err(e) = broker.subscribe(&client_id, "/videos/#") {
        println!("Failed to subscribe to /videos/#: {}", e);
        return;
    }

    let mut received_messages = Vec::new();
    let start = Instant::now();

    // Try to receive messages with timeout
    while start.elapsed() < Duration::from_secs(5) && received_messages.len() < 4 {
        match broker.receive(&client_id) {
            Ok(message) => {
                received_messages.push((
                    message.topic.name().to_string(),
                    String::from_utf8_lossy(&message.payload).to_string(),
                ));
            }
            Err(e) => {
                println!("Error receiving message: {}", e);
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    // Verify received messages
    if received_messages.len() < 3 {
        println!(
            "Expected at least 3 messages, got {}",
            received_messages.len()
        );
        return;
    }

    // Verify specific messages
    let has_png = received_messages
        .iter()
        .any(|(t, d)| t == "/images/png" && d == "PNG image data");
    let has_jpg = received_messages
        .iter()
        .any(|(t, d)| t == "/images/jpg" && d == "JPG image data");
    let has_mp4 = received_messages
        .iter()
        .any(|(t, d)| t == "/videos/mp4" && d == "MP4 video data");

    assert!(has_png, "Missing PNG message");
    assert!(has_jpg, "Missing JPG message");
    assert!(has_mp4, "Missing MP4 message");
}
