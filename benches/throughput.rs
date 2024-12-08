use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use roker::{Broker, BrokerConfig, Message, Topic};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

const KB: usize = 1024;
const MB: usize = 1024 * KB;

// Adjusted timeout constants
const OPERATION_TIMEOUT: Duration = Duration::from_millis(50);
const BENCHMARK_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_millis(25);
const GLOBAL_TIMEOUT: Duration = Duration::from_secs(30);

fn create_broker(size: usize) -> Arc<Broker> {
    let config = BrokerConfig {
        name: format!("bench_broker_{}", size),
        buffer_size: size,
        max_clients: 100,
        max_subscriptions_per_client: 10,
    };
    Arc::new(Broker::new(config).unwrap())
}

fn single_publisher_single_subscriber(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_pub_single_sub");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));
    group.warm_up_time(Duration::from_secs(1));

    // Test with a smaller buffer size
    for size in [1 * KB].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let data = vec![1u8; 32]; // 32 bytes of data
            let total_size = data.len() + std::mem::size_of::<u32>(); // Total size with length prefix

            eprintln!("Benchmark configuration:");
            eprintln!("  Buffer size: {} bytes", size);
            eprintln!("  Message size: {} bytes", data.len());
            eprintln!("  Total size with prefix: {} bytes", total_size);

            b.iter(|| {
                // Create a new broker for each iteration
                let broker = create_broker(size);
                let pub_id = broker.register_client("publisher").unwrap();
                let sub_id = broker.register_client("subscriber").unwrap();

                let topic = Topic::new("/test/topic").unwrap();
                broker.subscribe(&sub_id, "/test/topic").unwrap();

                eprintln!("\n=== New iteration ===");
                eprintln!("Buffer state before publish:");
                eprintln!("  Write position: {}", broker.debug_write_pos());
                eprintln!("  Read position: {}", broker.debug_read_pos());

                eprintln!("Publishing message...");
                broker
                    .publish(Message::new(topic.clone(), black_box(data.clone())))
                    .unwrap();

                eprintln!("Buffer state after publish:");
                eprintln!("  Write position: {}", broker.debug_write_pos());
                eprintln!("  Read position: {}", broker.debug_read_pos());

                let start = Instant::now();
                let deadline = start + Duration::from_millis(100);
                let mut received = false;
                let mut attempts = 0;

                while Instant::now() < deadline && !received {
                    attempts += 1;
                    match broker.receive(&sub_id) {
                        Ok(msg) => {
                            eprintln!("Message received successfully after {} attempts", attempts);
                            eprintln!("Buffer state after receive:");
                            eprintln!("  Write position: {}", broker.debug_write_pos());
                            eprintln!("  Read position: {}", broker.debug_read_pos());
                            assert_eq!(&msg.payload, &data);
                            received = true;
                        }
                        Err(e) => {
                            if attempts % 1000 == 0 {
                                eprintln!("Failed to receive (attempt {}): {:?}", attempts, e);
                                eprintln!("  Write position: {}", broker.debug_write_pos());
                                eprintln!("  Read position: {}", broker.debug_read_pos());
                            }
                            thread::yield_now();
                        }
                    }
                }

                if !received {
                    eprintln!(
                        "Failed to receive message within deadline after {} attempts",
                        attempts
                    );
                    eprintln!("Final buffer state:");
                    eprintln!("  Write position: {}", broker.debug_write_pos());
                    eprintln!("  Read position: {}", broker.debug_read_pos());
                }

                // Clean up
                drop(broker);
                thread::sleep(CLEANUP_TIMEOUT);
            });
        });
    }
    group.finish();
}

fn multiple_publishers_single_subscriber(c: &mut Criterion) {
    let mut group = c.benchmark_group("multi_pub_single_sub");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));
    group.warm_up_time(Duration::from_secs(1));

    for size in [64 * KB].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let broker = create_broker(size);
            let sub_id = broker.register_client("subscriber").unwrap();
            let _pub_id = broker.register_client("publisher").unwrap();

            let topic = Topic::new("/test/topic").unwrap();
            broker.subscribe(&sub_id, "/test/topic").unwrap();

            // Use smaller messages that fit in the buffer
            let data = vec![1u8; 32]; // Reduced from 64 to 32 bytes

            b.iter(|| {
                broker
                    .publish(Message::new(topic.clone(), black_box(data.clone())))
                    .unwrap();

                let start = Instant::now();
                let deadline = start + Duration::from_millis(100);
                while Instant::now() < deadline {
                    if broker.receive(&sub_id).is_ok() {
                        break;
                    }
                    thread::yield_now();
                }
            });

            drop(broker);
            thread::sleep(CLEANUP_TIMEOUT);
        });
    }
    group.finish();
}

fn single_publisher_multiple_subscribers(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_pub_multi_sub");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));
    group.warm_up_time(Duration::from_secs(1));

    for size in [64 * KB].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let broker = create_broker(size);
            let _pub_id = broker.register_client("publisher").unwrap();
            let sub_id1 = broker.register_client("subscriber1").unwrap();
            let sub_id2 = broker.register_client("subscriber2").unwrap();

            let topic = Topic::new("/test/topic").unwrap();
            broker.subscribe(&sub_id1, "/test/topic").unwrap();
            broker.subscribe(&sub_id2, "/test/topic").unwrap();

            // Use smaller messages that fit in the buffer
            let data = vec![1u8; 32]; // Reduced from 64 to 32 bytes

            b.iter(|| {
                broker
                    .publish(Message::new(topic.clone(), black_box(data.clone())))
                    .unwrap();

                let start = Instant::now();
                let deadline = start + Duration::from_millis(100);
                let mut received1 = false;
                let mut received2 = false;

                while Instant::now() < deadline && (!received1 || !received2) {
                    if !received1 {
                        if broker.receive(&sub_id1).is_ok() {
                            received1 = true;
                        }
                    }
                    if !received2 {
                        if broker.receive(&sub_id2).is_ok() {
                            received2 = true;
                        }
                    }
                    thread::yield_now();
                }
            });

            drop(broker);
            thread::sleep(CLEANUP_TIMEOUT);
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    single_publisher_single_subscriber,
    multiple_publishers_single_subscriber,
    single_publisher_multiple_subscribers
);
criterion_main!(benches);
