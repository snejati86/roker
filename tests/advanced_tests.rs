use shared_memory_broker::{Broker, BrokerConfig, ClientId, Message, Topic};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, info};
use tracing_test::traced_test;

const TEST_SHM_NAME: &str = "test_broker_advanced";
const BUFFER_SIZE: usize = 1024 * 1024; // 1MB

/// Performance thresholds
const MAX_PUBLISH_LATENCY_US: u64 = 50; // 50 microseconds
const MAX_SUBSCRIBE_LATENCY_US: u64 = 100; // 100 microseconds
const MIN_MESSAGES_PER_SEC: usize = 10_000; // 10k messages per second

#[traced_test]
#[test]
fn test_rapid_connect_disconnect() {
    let config = BrokerConfig {
        name: TEST_SHM_NAME.into(),
        buffer_size: BUFFER_SIZE,
        ..Default::default()
    };
    
    let broker = Arc::new(Broker::new(config).expect("Failed to create broker"));
    
    // Rapidly connect and disconnect multiple clients
    for i in 0..1000 {
        let client_id = broker.register_client(&format!("client{}", i))
            .expect("Failed to register client");
        broker.unregister_client(&client_id).expect("Failed to unregister client");
    }

    // Verify logs contain connection info
    assert!(logs_contain("Creating new broker"));
    assert!(logs_contain("Registering new client"));
}

#[traced_test]
#[test]
fn test_multiple_subscribers_same_topic() {
    let config = BrokerConfig {
        name: TEST_SHM_NAME.into(),
        buffer_size: BUFFER_SIZE,
        max_clients: 1000,
        max_subscriptions_per_client: 100,
    };
    
    let broker = Arc::new(Broker::new(config).expect("Failed to create broker"));
    let num_subscribers = 100;
    let message_count = 100;
    
    // Create multiple subscribers for the same topic
    let mut handles = vec![];
    let start_signal = Arc::new(AtomicBool::new(false));
    let received_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    
    for i in 0..num_subscribers {
        let broker = Arc::clone(&broker);
        let start = Arc::clone(&start_signal);
        let count = Arc::clone(&received_count);
        
        handles.push(thread::spawn(move || {
            let client_id = broker.register_client(&format!("client{}", i))
                .expect("Failed to register client");
            
            broker.subscribe(&client_id, "/test/topic")
                .expect("Failed to subscribe");
            
            while !start.load(Ordering::Relaxed) {
                thread::yield_now();
            }
            
            let mut received = 0;
            while received < message_count {
                if let Ok(_) = broker.receive(&client_id) {
                    received += 1;
                    count.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }
    
    // Wait for all subscribers to be ready
    thread::sleep(Duration::from_millis(100));
    start_signal.store(true, Ordering::Relaxed);
    
    // Create publisher
    let publisher_id = broker.register_client("publisher")
        .expect("Failed to register publisher");
    
    let topic = Topic::new("/test/topic").unwrap();
    
    // Publish messages
    for i in 0..message_count {
        let message = Message::new(
            topic.clone(),
            format!("message{}", i).into_bytes(),
        );
        broker.publish(message).expect("Failed to publish");
    }
    
    // Wait for all subscribers
    for handle in handles {
        handle.join().unwrap();
    }
    
    assert_eq!(
        received_count.load(Ordering::Relaxed),
        num_subscribers * message_count,
        "Not all messages were received"
    );
}

#[traced_test]
#[test]
fn test_performance_guarantees() {
    let config = BrokerConfig {
        name: TEST_SHM_NAME.into(),
        buffer_size: BUFFER_SIZE,
        ..Default::default()
    };
    
    let broker = Arc::new(Broker::new(config).expect("Failed to create broker"));
    let client_id = broker.register_client("perf_test")
        .expect("Failed to register client");
    
    let topic = Topic::new("/test/perf").unwrap();
    let data = vec![1u8; 1024]; // 1KB message
    
    // Test publish latency
    let start = Instant::now();
    let message = Message::new(topic.clone(), data.clone());
    broker.publish(message).expect("Failed to publish");
    let publish_latency = start.elapsed();
    
    assert!(
        publish_latency.as_micros() <= MAX_PUBLISH_LATENCY_US as u128,
        "Publish latency too high: {:?} > {:?}µs",
        publish_latency.as_micros(),
        MAX_PUBLISH_LATENCY_US
    );
    
    // Test subscribe and receive latency
    broker.subscribe(&client_id, "/test/perf").expect("Failed to subscribe");
    
    let start = Instant::now();
    broker.receive(&client_id).expect("Failed to receive");
    let receive_latency = start.elapsed();
    
    assert!(
        receive_latency.as_micros() <= MAX_SUBSCRIBE_LATENCY_US as u128,
        "Receive latency too high: {:?} > {:?}µs",
        receive_latency.as_micros(),
        MAX_SUBSCRIBE_LATENCY_US
    );
    
    // Test throughput
    let message_count = 10_000;
    let start = Instant::now();
    
    for i in 0..message_count {
        let message = Message::new(topic.clone(), data.clone());
        broker.publish(message).expect("Failed to publish");
        if i % 100 == 0 {
            thread::yield_now();
        }
    }
    
    let elapsed = start.elapsed();
    let messages_per_sec = message_count as f64 / elapsed.as_secs_f64();
    
    assert!(
        messages_per_sec >= MIN_MESSAGES_PER_SEC as f64,
        "Throughput too low: {:.2} msg/s < {} msg/s",
        messages_per_sec,
        MIN_MESSAGES_PER_SEC
    );

    // Verify performance logs
    assert!(logs_contain("Publishing message"));
    assert!(logs_contain("Delivering message"));
}

#[traced_test]
#[test]
fn test_topic_edge_cases() {
    let config = BrokerConfig {
        name: TEST_SHM_NAME.into(),
        buffer_size: BUFFER_SIZE,
        ..Default::default()
    };
    
    let broker = Arc::new(Broker::new(config).expect("Failed to create broker"));
    let client_id = broker.register_client("edge_test")
        .expect("Failed to register client");
    
    // Test empty topic
    assert!(Topic::new("").is_err());
    
    // Test very long topic
    let long_topic = "/".repeat(shared_memory_broker::MAX_TOPIC_LENGTH + 1);
    assert!(Topic::new(&long_topic).is_err());
    
    // Test special characters in topic
    broker.subscribe(&client_id, "/test/+/#/*/").expect("Failed to subscribe");
    let topic = Topic::new("/test/+/#/*/").unwrap();
    let message = Message::new(topic, b"data".to_vec());
    broker.publish(message).expect("Failed to publish");
    
    // Test nested wildcards
    broker.subscribe(&client_id, "/test/#/#").expect("Failed to subscribe");
    broker.subscribe(&client_id, "/test/*/*/#").expect("Failed to subscribe");
}

#[traced_test]
#[test]
fn test_broker_stats() {
    let config = BrokerConfig {
        name: TEST_SHM_NAME.into(),
        buffer_size: BUFFER_SIZE,
        ..Default::default()
    };
    
    let broker = Arc::new(Broker::new(config).expect("Failed to create broker"));
    
    // Register some clients
    let client1 = broker.register_client("client1").unwrap();
    let client2 = broker.register_client("client2").unwrap();
    
    // Add some subscriptions
    broker.subscribe(&client1, "/test/#").unwrap();
    broker.subscribe(&client2, "/test/specific").unwrap();
    
    // Publish some messages
    let topic = Topic::new("/test/data").unwrap();
    let message = Message::new(topic, b"test".to_vec());
    broker.publish(message).unwrap();
    
    // Get stats
    let stats = broker.stats();
    
    assert_eq!(stats.connected_clients, 2);
    assert_eq!(stats.total_subscriptions, 2);
    assert_eq!(stats.messages_published, 1);
    assert!(stats.buffer_usage >= 0.0 && stats.buffer_usage <= 1.0);
}

#[traced_test]
#[test]
fn test_cleanup_on_drop() {
    let config = BrokerConfig {
        name: TEST_SHM_NAME.into(),
        buffer_size: BUFFER_SIZE,
        ..Default::default()
    };
    
    let broker = Arc::new(Broker::new(config.clone()).expect("Failed to create broker"));
    let client_id = broker.register_client("cleanup_test").unwrap();
    
    // Create some messages
    let topic = Topic::new("/test/cleanup").unwrap();
    for i in 0..100 {
        let message = Message::new(
            topic.clone(),
            format!("{}", i).into_bytes(),
        );
        broker.publish(message).unwrap();
    }
    
    // Drop the broker
    drop(broker);
    
    // Try to connect to the same shared memory segment
    let new_broker = Broker::new(config);
    assert!(new_broker.is_ok(), "Failed to create new broker after cleanup");
    
    // Verify cleanup logs
    assert!(logs_contain("Dropping ring buffer"));
} 