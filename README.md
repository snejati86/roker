# Roker

A high-performance shared memory message broker written in Rust. Roker (Rust + Broker) provides a fast and efficient way to implement inter-process communication using a publish/subscribe pattern with shared memory.

[![Crates.io](https://img.shields.io/crates/v/roker.svg)](https://crates.io/crates/roker)
[![Documentation](https://docs.rs/roker/badge.svg)](https://docs.rs/roker)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

## Features

- Fast shared memory-based communication
- Topic-based publish/subscribe pattern with wildcard support
- Thread-safe and process-safe implementation
- Configurable buffer sizes and client limits
- Comprehensive error handling and logging
- Zero-copy message passing where possible
- Async support with Tokio

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
roker = "0.1.0"
```

## Quick Start

```rust
use roker::{Publisher, Subscriber, Topic, Message};

// Process 1: Create a publisher
let publisher = Publisher::connect("my_broker").expect("Failed to connect publisher");

// Process 2: Create a subscriber
let subscriber = Subscriber::connect("my_broker").expect("Failed to connect subscriber");

// Subscribe to a topic pattern
subscriber.subscribe("/sensors/#").expect("Failed to subscribe");

// Publish a message
let topic = Topic::new("/sensors/temperature").expect("Invalid topic");
let message = Message::new(topic, b"25.5".to_vec());
publisher.publish(&topic, b"25.5").expect("Failed to publish");

// Receive messages
if let Ok(message) = subscriber.receive() {
    println!("Received: {:?}", message);
}
```

## High-Level APIs

### Publisher API
```rust
// Create a publisher
let publisher = Publisher::connect("my_broker")?;

// Publish single message
publisher.publish(&topic, data)?;

// Publish multiple messages
publisher.publish_batch(&[(&topic1, data1), (&topic2, data2)])?;
```

### Subscriber API
```rust
// Create a subscriber
let subscriber = Subscriber::connect("my_broker")?;

// Subscribe to topics (supports wildcards)
subscriber.subscribe("/sensors/#")?;
subscriber.unsubscribe("/sensors/temperature")?;

// Receive messages
let message = subscriber.receive()?;
let messages = subscriber.receive_batch(10)?;
```

### Configuration
```rust
let config = BrokerConfig {
    name: "my_broker".to_string(),
    buffer_size: 64 * 1024 * 1024, // 64MB
    max_clients: 1000,
    max_subscriptions_per_client: 100,
};
```

## Topic Patterns

The broker supports wildcard patterns in topic subscriptions:

- `*`: Matches any single level
- `#`: Matches multiple levels
- Example: `/sensors/*/temperature` matches `/sensors/room1/temperature`
- Example: `/sensors/#` matches all topics under `/sensors/`

## Performance

The broker is designed for high-performance scenarios:

- Message throughput: Up to 1M messages/second (depending on message size and hardware)
- Latency: Sub-microsecond in optimal conditions
- Memory efficient: Zero-copy message passing where possible
- Scalable: Supports thousands of concurrent clients

## Error Handling

The library uses the `thiserror` crate for comprehensive error handling:

```rust
use roker::Error;

match publisher.publish(&topic, data) {
    Ok(_) => println!("Message published"),
    Err(Error::BufferFull) => println!("Buffer is full"),
    Err(e) => eprintln!("Error: {}", e),
}
```

## Documentation

- [API Documentation](https://snejati86.github.io/roker)
- [Examples](examples/)
- [Contributing Guide](CONTRIBUTING.md)

## Examples

Check the [examples](examples/) directory for more detailed usage examples:

- Image broadcasting between processes
- Temperature telemetry system
- Topic patterns and wildcards
- Error handling
- Performance benchmarks

## Contributing

Contributions are welcome! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

## License

This project is licensed under
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)