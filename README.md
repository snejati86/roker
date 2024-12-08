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
use roker::{Broker, BrokerConfig, Message, Topic};
use std::sync::Arc;

// Create a broker with default configuration
let config = BrokerConfig::default();
let broker = Arc::new(Broker::new(config).expect("Failed to create broker"));

// Register clients
let publisher_id = broker.register_client("publisher").expect("Failed to register publisher");
let subscriber_id = broker.register_client("subscriber").expect("Failed to register subscriber");

// Subscribe to a topic
broker.subscribe(&subscriber_id, "/sensors/#").expect("Failed to subscribe");

// Publish a message
let topic = Topic::new("/sensors/temperature").expect("Invalid topic");
let message = Message::new(topic, b"25.5".to_vec());
broker.publish(message).expect("Failed to publish");

// Receive messages
if let Ok(message) = broker.receive(&subscriber_id) {
    println!("Received: {:?}", message);
}
```

## Performance

The broker is designed for high-performance scenarios:

- Message throughput: Up to 1M messages/second (depending on message size and hardware)
- Latency: Sub-microsecond in optimal conditions
- Memory efficient: Zero-copy message passing where possible
- Scalable: Supports thousands of concurrent clients

## Configuration

The broker can be configured through the `BrokerConfig` struct:

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

## Error Handling

The library uses the `thiserror` crate for comprehensive error handling:

```rust
use roker::Error;

match broker.publish(message) {
    Ok(_) => println!("Message published"),
    Err(Error::BufferFull) => println!("Buffer is full"),
    Err(e) => eprintln!("Error: {}", e),
}
```

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

This project is licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option. 