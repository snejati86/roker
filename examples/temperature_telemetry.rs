use rand::Rng;
use roker::{Broker, BrokerConfig, ClientId, Message, Topic};
use serde::{Deserialize, Serialize};
use std::env;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BROKER_NAME: &str = "telemetry_broker";
const BUFFER_SIZE: usize = 1048576; // 1MB (2^20)

#[derive(Serialize, Deserialize, Debug)]
struct TemperatureReading {
    sensor_id: String,
    temperature: f64,
    timestamp: u64,
    location: String,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        println!("Usage:");
        println!("  Sensor: {} sensor <sensor_id> <location>", args[0]);
        println!("  Monitor: {} monitor <alert_threshold>", args[0]);
        return;
    }

    match args[1].as_str() {
        "sensor" => {
            if args.len() < 4 {
                println!("Sensor requires sensor_id and location arguments");
                return;
            }
            run_sensor(&args[2], &args[3]);
        }
        "monitor" => {
            if args.len() < 3 {
                println!("Monitor requires alert_threshold argument");
                return;
            }
            let threshold = args[2].parse().unwrap_or(30.0);
            run_monitor(threshold);
        }
        _ => println!("Unknown role. Use 'sensor' or 'monitor'"),
    }
}

fn run_sensor(sensor_id: &str, location: &str) {
    // Connect to or create broker
    let config = BrokerConfig {
        name: BROKER_NAME.to_string(),
        buffer_size: BUFFER_SIZE,
        max_clients: 10,
        max_subscriptions_per_client: 5,
    };

    let broker = match Broker::new(config) {
        Ok(b) => b,
        Err(e) => {
            println!("Failed to create broker: {}", e);
            return;
        }
    };

    println!("Temperature sensor started");
    println!("ID: {}", sensor_id);
    println!("Location: {}", location);

    let mut rng = rand::thread_rng();
    let topic =
        Topic::new(&format!("/temperature/{}/{}", location, sensor_id)).expect("Invalid topic");

    // Simulate temperature readings
    loop {
        // Generate a somewhat realistic temperature (15-35°C with some random variation)
        let base_temp = 25.0;
        let variation = rng.gen_range(-10.0..10.0);
        let temperature = base_temp + variation;

        let reading = TemperatureReading {
            sensor_id: sensor_id.to_string(),
            temperature,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            location: location.to_string(),
        };

        // Serialize the reading
        let payload = serde_json::to_vec(&reading).unwrap();
        let message = Message::new(topic.clone(), payload);

        // Publish the reading
        match broker.publish(message) {
            Ok(_) => println!("Published temperature: {:.1}°C", temperature),
            Err(e) => println!("Failed to publish temperature: {}", e),
        }

        // Wait before next reading
        thread::sleep(Duration::from_secs(2));
    }
}

fn run_monitor(alert_threshold: f64) {
    // Connect to broker
    let broker = match Broker::connect(BROKER_NAME) {
        Ok(b) => b,
        Err(e) => {
            println!("Failed to connect to broker: {}", e);
            return;
        }
    };

    // Register as a client
    let client_id = match broker.register_client("temperature_monitor") {
        Ok(id) => id,
        Err(e) => {
            println!("Failed to register client: {}", e);
            return;
        }
    };

    // Subscribe to all temperature topics
    if let Err(e) = broker.subscribe(&client_id, "/temperature/#") {
        println!("Failed to subscribe: {}", e);
        return;
    }

    println!("Temperature monitor started");
    println!("Alert threshold: {:.1}°C", alert_threshold);

    // Monitor temperatures
    loop {
        match broker.receive(&client_id) {
            Ok(message) => {
                match serde_json::from_slice::<TemperatureReading>(&message.payload) {
                    Ok(reading) => {
                        println!(
                            "Sensor {} at {} - Temperature: {:.1}°C",
                            reading.sensor_id, reading.location, reading.temperature
                        );

                        // Check if temperature exceeds threshold
                        if reading.temperature > alert_threshold {
                            println!(
                                "⚠️  ALERT: High temperature detected! {:.1}°C at {} (Sensor: {})",
                                reading.temperature, reading.location, reading.sensor_id
                            );
                        }
                    }
                    Err(e) => println!("Failed to parse temperature reading: {}", e),
                }
            }
            Err(e) => {
                println!("Error receiving message: {}", e);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}
