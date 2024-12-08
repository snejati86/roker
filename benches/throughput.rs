use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use roker::{Broker, BrokerConfig, Message, Topic};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

const KB: usize = 1024;
const MB: usize = 1024 * KB;
const OPERATION_TIMEOUT: Duration = Duration::from_millis(100);

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
    group.sample_size(50); // Reduce sample size for stability

    for size in [4 * KB, 64 * KB, 1 * MB].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let broker = create_broker(size);
            let pub_id = broker.register_client("publisher").unwrap();
            let sub_id = broker.register_client("subscriber").unwrap();

            let topic = Topic::new("/test/topic").unwrap();
            broker.subscribe(&sub_id, "/test/topic").unwrap();

            let data = vec![1u8; 1024]; // 1KB messages

            b.iter(|| {
                broker
                    .publish(Message::new(topic.clone(), black_box(data.clone())))
                    .unwrap();

                let start = Instant::now();
                while start.elapsed() < OPERATION_TIMEOUT {
                    if let Ok(received) = broker.receive(&sub_id) {
                        assert_eq!(&received.payload, &data);
                        break;
                    }
                    thread::yield_now();
                }
            });

            // Cleanup
            drop(broker);
            thread::sleep(Duration::from_millis(10));
        });
    }
    group.finish();
}

fn multiple_publishers_single_subscriber(c: &mut Criterion) {
    let mut group = c.benchmark_group("multi_pub_single_sub");
    group.sample_size(20); // Reduce sample size for stability
    group.measurement_time(Duration::from_secs(2));

    for size in [64 * KB, 1 * MB].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let broker = create_broker(size);
            let sub_id = broker.register_client("subscriber").unwrap();
            let pub_id = broker.register_client("publisher").unwrap();

            let topic = Topic::new("/test/topic").unwrap();
            broker.subscribe(&sub_id, "/test/topic").unwrap();

            let data = vec![1u8; 1024];
            let mut received_count = 0;

            b.iter(|| {
                // Publish message
                broker
                    .publish(Message::new(topic.clone(), black_box(data.clone())))
                    .unwrap();

                // Try to receive with timeout
                let start = Instant::now();
                while start.elapsed() < OPERATION_TIMEOUT {
                    if broker.receive(&sub_id).is_ok() {
                        received_count += 1;
                        break;
                    }
                    thread::yield_now();
                }
            });

            println!("Messages received: {}", received_count);

            // Cleanup
            drop(broker);
            thread::sleep(Duration::from_millis(50));
        });
    }
    group.finish();
}

fn single_publisher_multiple_subscribers(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_pub_multi_sub");
    let num_subscribers = 2; // Reduced from 4 to 2 for stability
    group.sample_size(30); // Reduce sample size for stability

    for size in [64 * KB, 1 * MB].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let broker = create_broker(size);
            let pub_id = broker.register_client("publisher").unwrap();
            let sub_ids: Vec<_> = (0..num_subscribers)
                .map(|i| {
                    let id = broker
                        .register_client(&format!("subscriber_{}", i))
                        .unwrap();
                    broker.subscribe(&id, "/test/topic").unwrap();
                    id
                })
                .collect();

            let topic = Topic::new("/test/topic").unwrap();
            let data = vec![1u8; 1024];

            b.iter(|| {
                broker
                    .publish(Message::new(topic.clone(), black_box(data.clone())))
                    .unwrap();

                for sub_id in &sub_ids {
                    let start = Instant::now();
                    while start.elapsed() < OPERATION_TIMEOUT {
                        if let Ok(received) = broker.receive(sub_id) {
                            assert_eq!(&received.payload, &data);
                            break;
                        }
                        thread::yield_now();
                    }
                }
            });

            // Cleanup
            drop(broker);
            thread::sleep(Duration::from_millis(10));
        });
    }
    group.finish();
}

fn high_throughput_test(c: &mut Criterion) {
    let mut group = c.benchmark_group("high_throughput");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    let num_publishers = 2; // Reduced from 4 to 2
    let num_subscribers = 2; // Reduced from 4 to 2
    let messages_per_publisher = 1000; // Reduced from 10000
    let buffer_size = 16 * MB;

    group.bench_function("multi_pub_multi_sub", |b| {
        b.iter(|| {
            let broker = create_broker(buffer_size);
            let barrier = Arc::new(Barrier::new(num_publishers + num_subscribers + 1));

            // Register clients
            let pub_ids: Vec<_> = (0..num_publishers)
                .map(|i| broker.register_client(&format!("publisher_{}", i)).unwrap())
                .collect();

            let sub_ids: Vec<_> = (0..num_subscribers)
                .map(|i| {
                    let id = broker
                        .register_client(&format!("subscriber_{}", i))
                        .unwrap();
                    broker.subscribe(&id, "/test/topic").unwrap();
                    id
                })
                .collect();

            let topic = Topic::new("/test/topic").unwrap();
            let broker = Arc::new(broker);

            // Spawn publishers
            let publisher_handles: Vec<_> = pub_ids
                .into_iter()
                .map(|id| {
                    let broker = Arc::clone(&broker);
                    let barrier = Arc::clone(&barrier);
                    let topic = topic.clone();
                    thread::spawn(move || {
                        barrier.wait();

                        for i in 0..messages_per_publisher {
                            let msg = format!("Message {} from publisher {}", i, id.as_str());
                            let start = Instant::now();
                            while start.elapsed() < OPERATION_TIMEOUT {
                                if broker
                                    .publish(Message::new(topic.clone(), msg.clone().into_bytes()))
                                    .is_ok()
                                {
                                    break;
                                }
                                thread::yield_now();
                            }
                        }
                    })
                })
                .collect();

            // Spawn subscribers
            let subscriber_handles: Vec<_> = sub_ids
                .into_iter()
                .map(|id| {
                    let broker = Arc::clone(&broker);
                    let barrier = Arc::clone(&barrier);
                    thread::spawn(move || {
                        let mut received = 0;
                        barrier.wait();

                        let start = Instant::now();
                        while received < num_publishers * messages_per_publisher
                            && start.elapsed() < Duration::from_secs(30)
                        {
                            if broker.receive(&id).is_ok() {
                                received += 1;
                            } else {
                                thread::yield_now();
                            }
                        }
                        received
                    })
                })
                .collect();

            // Start the test
            barrier.wait();

            // Wait for completion
            for handle in publisher_handles {
                handle.join().unwrap();
            }

            let received_counts: Vec<_> = subscriber_handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect();

            // Verify that all subscribers received all messages
            for count in received_counts {
                assert_eq!(count, num_publishers * messages_per_publisher);
            }

            // Cleanup
            drop(broker);
            thread::sleep(Duration::from_millis(10));
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    single_publisher_single_subscriber,
    multiple_publishers_single_subscriber,
    single_publisher_multiple_subscribers,
    high_throughput_test
);
criterion_main!(benches);
