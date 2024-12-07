use shared_memory_broker::{Broker, BrokerConfig, ClientId, Message, Topic};
use std::process::Command;
use std::time::Duration;
use std::{env, thread};

const TEST_SHM_NAME: &str = "test_broker";
const BUFFER_SIZE: usize = 1024 * 1024; // 1MB

#[test]
fn test_ipc_communication() {
    let config = BrokerConfig {
        name: TEST_SHM_NAME.to_string(),
        buffer_size: BUFFER_SIZE,
        max_clients: 100,
        max_subscriptions_per_client: 10,
    };
    let _broker = Broker::new(config).expect("Failed to create broker");
    
    // Start publisher process
    let publisher = thread::spawn(|| {
        let status = Command::new(env::current_exe().unwrap())
            .arg("--test-threads=1")
            .env("TEST_PUBLISHER", "1")
            .status()
            .expect("Failed to start publisher process");
        assert!(status.success());
    });

    // Start subscriber process
    let subscriber = thread::spawn(|| {
        let status = Command::new(env::current_exe().unwrap())
            .arg("--test-threads=1")
            .env("TEST_SUBSCRIBER", "1")
            .status()
            .expect("Failed to start subscriber process");
        assert!(status.success());
    });

    // Give processes time to start
    thread::sleep(Duration::from_millis(100));

    publisher.join().expect("Publisher process failed");
    subscriber.join().expect("Subscriber process failed");
}

#[test]
fn publisher_process() {
    if env::var("TEST_PUBLISHER").is_err() {
        return;
    }

    // Give the main process time to create the broker
    thread::sleep(Duration::from_millis(200));

    let broker = Broker::connect(TEST_SHM_NAME).expect("Failed to connect to broker");
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
        broker.publish(message).expect("Failed to publish");
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn subscriber_process() {
    if env::var("TEST_SUBSCRIBER").is_err() {
        return;
    }

    // Give the main process time to create the broker
    thread::sleep(Duration::from_millis(200));

    let broker = Broker::connect(TEST_SHM_NAME).expect("Failed to connect to broker");
    let client_id = ClientId::new();
    
    // Subscribe to different patterns
    broker.subscribe(&client_id, "/images/*").expect("Failed to subscribe");
    broker.subscribe(&client_id, "/videos/#").expect("Failed to subscribe");

    let mut received_messages = Vec::new();

    // Try to receive messages for a while
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(message) = broker.receive(&client_id) {
            received_messages.push((message.topic.name().to_string(), 
                                 String::from_utf8_lossy(&message.payload).to_string()));
        }
        thread::sleep(Duration::from_millis(100));
    }

    // Verify received messages
    assert!(received_messages.len() >= 3, "Expected at least 3 messages, got {}", received_messages.len());
    
    // Verify specific messages
    let has_png = received_messages.iter().any(|(t, d)| t == "/images/png" && d == "PNG image data");
    let has_jpg = received_messages.iter().any(|(t, d)| t == "/images/jpg" && d == "JPG image data");
    let has_mp4 = received_messages.iter().any(|(t, d)| t == "/videos/mp4" && d == "MP4 video data");
    
    assert!(has_png, "Missing PNG message");
    assert!(has_jpg, "Missing JPG message");
    assert!(has_mp4, "Missing MP4 message");

    println!("Received messages: {:?}", received_messages);
}
