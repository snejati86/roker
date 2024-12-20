use roker::{Broker, BrokerConfig, Message, Topic};
use std::thread;
use std::time::{Duration, Instant};

// Timeout wrapper function
fn run_with_timeout<T, F>(timeout: Duration, test_fn: F) -> T
where
    F: FnOnce() -> T,
{
    let start = Instant::now();
    let result = test_fn();

    if start.elapsed() > timeout {
        panic!("Test exceeded timeout of {:?}", timeout);
    }

    result
}

#[test]
fn test_basic_functionality() {
    run_with_timeout(Duration::from_secs(5), || {
        println!("Starting basic functionality test");

        // Create broker with minimal configuration
        let config = BrokerConfig {
            name: "broker1".to_string(),
            buffer_size: 8192,
            max_clients: 10,
            max_subscriptions_per_client: 5,
        };

        println!("Creating broker");
        let broker = match Broker::new(config) {
            Ok(b) => b,
            Err(e) => {
                println!("Failed to create broker: {:?}", e);
                panic!("Broker creation failed: {}", e);
            }
        };

        println!("Registering client");
        let client_id = match broker.register_client("test_client") {
            Ok(id) => id,
            Err(e) => {
                println!("Failed to register client: {:?}", e);
                panic!("Client registration failed: {}", e);
            }
        };

        println!("Subscribing to topic");
        if let Err(e) = broker.subscribe(&client_id, "/test/topic") {
            println!("Failed to subscribe: {:?}", e);
            panic!("Subscription failed: {}", e);
        }

        // Give a bit of time for subscription to be processed
        thread::sleep(Duration::from_millis(10));

        println!("Creating and publishing message");
        let topic = Topic::new("/test/topic").expect("Failed to create topic");
        let message = Message::new(topic, b"test message".to_vec());

        if let Err(e) = broker.publish(message.clone()) {
            println!("Failed to publish message: {:?}", e);
            panic!("Message publication failed: {}", e);
        }

        // Give a bit of time for message to be processed
        thread::sleep(Duration::from_millis(10));

        println!("Receiving message");
        let mut receive_success = false;
        for _ in 0..10 {
            match broker.receive(&client_id) {
                Ok(received) => {
                    assert_eq!(received.topic.name(), "/test/topic");
                    assert_eq!(received.payload, b"test message");
                    println!("Message received successfully");
                    receive_success = true;
                    break;
                }
                Err(e) => {
                    println!("Failed to receive message: {:?}, retrying...", e);
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
        assert!(
            receive_success,
            "Failed to receive message after multiple attempts"
        );

        println!("Test completed successfully");
    });
}

#[test]
fn test_cleanup() {
    run_with_timeout(Duration::from_secs(5), || {
        println!("Starting cleanup test");

        let name = "broker2".to_string();
        let config = BrokerConfig {
            name: name.clone(),
            buffer_size: 8192,
            max_clients: 10,
            max_subscriptions_per_client: 5,
        };

        println!("Creating first broker");
        let broker = match Broker::new(config.clone()) {
            Ok(b) => b,
            Err(e) => {
                println!("Failed to create first broker: {:?}", e);
                panic!("First broker creation failed: {}", e);
            }
        };

        // Do a simple operation to verify it works
        println!("Testing first broker");
        let client_id = broker
            .register_client("test")
            .expect("Failed to register client");
        broker
            .subscribe(&client_id, "/test")
            .expect("Failed to subscribe");

        // Drop the broker explicitly
        println!("Dropping first broker");
        drop(broker);

        // Give some time for cleanup
        println!("Waiting for cleanup");
        thread::sleep(Duration::from_millis(100));

        // Try to create a new broker with the same name
        println!("Creating second broker");
        match Broker::new(config) {
            Ok(_) => println!("Second broker created successfully"),
            Err(e) => {
                println!("Failed to create second broker: {:?}", e);
                panic!("Second broker creation failed: {}", e);
            }
        }

        println!("Test completed successfully");
    });
}
