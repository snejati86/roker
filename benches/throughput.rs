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

    // Test only one size for consistency
    for size in [64 * KB].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let broker = create_broker(size);
            let _pub_id = broker.register_client("publisher").unwrap();
            let sub_id = broker.register_client("subscriber").unwrap();

            let topic = Topic::new("/test/topic").unwrap();
            broker.subscribe(&sub_id, "/test/topic").unwrap();

            let data = vec![1u8; 64]; // Use smaller messages for consistency

            b.iter(|| {
                broker
                    .publish(Message::new(topic.clone(), black_box(data.clone())))
                    .unwrap();

                let start = Instant::now();
                let deadline = start + Duration::from_millis(100);
                while Instant::now() < deadline {
                    if let Ok(received) = broker.receive(&sub_id) {
                        assert_eq!(&received.payload, &data);
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

            let data = vec![1u8; 64];

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

    for size in [256 * KB].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let broker = create_broker(size);
            let _pub_id = broker.register_client("publisher").unwrap();

            let sub_ids: Vec<_> = (0..2)
                .map(|i| {
                    let id = broker
                        .register_client(&format!("subscriber_{}", i))
                        .unwrap();
                    broker.subscribe(&id, "/test/topic").unwrap();
                    id
                })
                .collect();

            let topic = Topic::new("/test/topic").unwrap();
            let data = vec![1u8; 64];

            b.iter(|| {
                broker
                    .publish(Message::new(topic.clone(), black_box(data.clone())))
                    .unwrap();

                let start = Instant::now();
                let mut received = vec![false; sub_ids.len()];
                let deadline = start + Duration::from_millis(100);

                while !received.iter().all(|&x| x) && Instant::now() < deadline {
                    for (idx, sub_id) in sub_ids.iter().enumerate() {
                        if !received[idx] {
                            if let Ok(msg) = broker.receive(sub_id) {
                                if msg.payload == data {
                                    received[idx] = true;
                                }
                            }
                        }
                    }
                    thread::yield_now();
                }

                assert!(
                    received.iter().all(|&x| x),
                    "Not all subscribers received the message"
                );
            });

            drop(broker);
            thread::sleep(CLEANUP_TIMEOUT);
        });
    }
    group.finish();
}

fn high_throughput_test(c: &mut Criterion) {
    let mut group = c.benchmark_group("high_throughput");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));
    group.warm_up_time(Duration::from_secs(1));

    let num_publishers = 2;
    let num_subscribers = 2;
    let messages_per_publisher = 100;
    let buffer_size = 4 * MB;

    group.bench_function("multi_pub_multi_sub", |b| {
        b.iter(|| {
            let broker = create_broker(buffer_size);
            let barrier = Arc::new(Barrier::new(num_publishers + num_subscribers + 1));

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
                            && start.elapsed() < BENCHMARK_TIMEOUT
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

            barrier.wait();

            for handle in publisher_handles {
                handle.join().expect("Publisher thread panicked");
            }

            let received_counts: Vec<_> = subscriber_handles
                .into_iter()
                .map(|h| h.join().expect("Subscriber thread panicked"))
                .collect();

            for count in received_counts {
                assert_eq!(count, num_publishers * messages_per_publisher);
            }

            drop(broker);
            thread::sleep(CLEANUP_TIMEOUT);
        })
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .measurement_time(Duration::from_secs(5))
        .warm_up_time(Duration::from_secs(1));
    targets = single_publisher_single_subscriber,
             multiple_publishers_single_subscriber,
             single_publisher_multiple_subscribers,
             high_throughput_test
}
criterion_main!(benches);
