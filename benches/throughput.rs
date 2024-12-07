use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use shared_memory_broker::{Publisher, RingBuffer, Subscriber};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

const KB: usize = 1024;
const MB: usize = 1024 * KB;

fn single_publisher_single_subscriber(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_pub_single_sub");

    for size in [4 * KB, 64 * KB, 1 * MB].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let buffer = Arc::new(RingBuffer::new(size).unwrap());
            let publisher = Publisher::new(Arc::clone(&buffer));
            let subscriber = Subscriber::new(Arc::clone(&buffer));
            let data = vec![1u8; 1024]; // 1KB messages
            let mut read_buf = vec![0u8; 1024];

            b.iter(|| {
                publisher.publish(black_box(&data)).unwrap();
                subscriber.receive(black_box(&mut read_buf)).unwrap();
                assert_eq!(&read_buf, &data);
            });
        });
    }
    group.finish();
}

fn multiple_publishers_single_subscriber(c: &mut Criterion) {
    let mut group = c.benchmark_group("multi_pub_single_sub");
    let num_publishers = 4;

    for size in [64 * KB, 1 * MB].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let buffer = Arc::new(RingBuffer::new(size).unwrap());
            let subscriber = Arc::new(Subscriber::new(Arc::clone(&buffer)));
            let publishers: Vec<_> = (0..num_publishers)
                .map(|_| Publisher::new(Arc::clone(&buffer)))
                .collect();

            let data = vec![1u8; 1024];
            let mut read_buf = vec![0u8; 1024];

            b.iter(|| {
                for publisher in &publishers {
                    publisher.publish(black_box(&data)).unwrap();
                }

                for _ in 0..num_publishers {
                    while subscriber.receive(black_box(&mut read_buf)).is_err() {
                        thread::yield_now();
                    }
                    assert_eq!(&read_buf, &data);
                }
            });
        });
    }
    group.finish();
}

fn single_publisher_multiple_subscribers(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_pub_multi_sub");
    let num_subscribers = 4;

    for size in [64 * KB, 1 * MB].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let buffer = Arc::new(RingBuffer::new(size).unwrap());
            let publisher = Publisher::new(Arc::clone(&buffer));
            let subscribers: Vec<_> = (0..num_subscribers)
                .map(|_| Subscriber::new(Arc::clone(&buffer)))
                .collect();

            let data = vec![1u8; 1024];
            let mut read_bufs: Vec<_> = (0..num_subscribers).map(|_| vec![0u8; 1024]).collect();

            b.iter(|| {
                publisher.publish(black_box(&data)).unwrap();

                for (subscriber, read_buf) in subscribers.iter().zip(read_bufs.iter_mut()) {
                    while subscriber.receive(black_box(read_buf)).is_err() {
                        thread::yield_now();
                    }
                    assert_eq!(read_buf, &data);
                }
            });
        });
    }
    group.finish();
}

fn high_throughput_test(c: &mut Criterion) {
    let mut group = c.benchmark_group("high_throughput");
    group.sample_size(10); // Reduce sample size for long-running benchmarks
    group.measurement_time(Duration::from_secs(10));

    let num_publishers = 4;
    let num_subscribers = 4;
    let messages_per_publisher = 10000;
    let buffer_size = 16 * MB;

    group.bench_function("multi_pub_multi_sub", |b| {
        b.iter(|| {
            let buffer = Arc::new(RingBuffer::new(buffer_size).unwrap());
            let barrier = Arc::new(Barrier::new(num_publishers + num_subscribers + 1));

            // Spawn publishers
            let publisher_handles: Vec<_> = (0..num_publishers)
                .map(|id| {
                    let buffer = Arc::clone(&buffer);
                    let barrier = Arc::clone(&barrier);
                    thread::spawn(move || {
                        let publisher = Publisher::new(buffer);
                        let data = format!("Message from publisher {}", id);
                        barrier.wait();

                        for i in 0..messages_per_publisher {
                            let msg = format!("{} - {}", data, i);
                            while publisher.publish(msg.as_bytes()).is_err() {
                                thread::yield_now();
                            }
                        }
                    })
                })
                .collect();

            // Spawn subscribers
            let subscriber_handles: Vec<_> = (0..num_subscribers)
                .map(|_| {
                    let buffer = Arc::clone(&buffer);
                    let barrier = Arc::clone(&barrier);
                    thread::spawn(move || {
                        let subscriber = Subscriber::new(buffer);
                        let mut received = 0;
                        let mut buf = vec![0u8; 1024];
                        barrier.wait();

                        while received < num_publishers * messages_per_publisher {
                            if subscriber.receive(&mut buf).is_ok() {
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
