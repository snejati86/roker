use roker::{Broker, BrokerConfig, ClientId, Message, Topic};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const BROKER_NAME: &str = "image_broker2";
const BUFFER_SIZE: usize = 1024 * 1024 * 16; // 16MB (2^24)
const IMAGE_TOPIC: &str = "/images/broadcast";

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        println!("Usage:");
        println!("  Publisher: {} publisher <image_path>", args[0]);
        println!("  Viewer: {} viewer <save_dir> <viewer_id>", args[0]);
        return;
    }

    match args[1].as_str() {
        "publisher" => run_publisher(&args[2]),
        "viewer" => {
            if args.len() < 4 {
                println!("Viewer requires save_dir and viewer_id arguments");
                return;
            }
            run_viewer(&args[2], &args[3])
        }
        _ => println!("Unknown role. Use 'publisher' or 'viewer'"),
    }
}

fn run_publisher(image_path: &str) {
    // Create broker configuration
    let config = BrokerConfig {
        name: BROKER_NAME.to_string(),
        buffer_size: BUFFER_SIZE,
        max_clients: 10,
        max_subscriptions_per_client: 5,
    };

    // Create or connect to broker
    let broker = match Broker::new(config) {
        Ok(b) => b,
        Err(e) => {
            println!("Failed to create broker: {}", e);
            return;
        }
    };
    println!("Broker created with buffer size");

    // Read image file
    let image_data = match fs::read(image_path) {
        Ok(data) => data,
        Err(e) => {
            println!("Failed to read image file: {}", e);
            return;
        }
    };

    println!("Publisher started. Broadcasting image: {}", image_path);
    println!("Image size: {} bytes", image_data.len());

    // Create topic and message
    let topic = Topic::new(IMAGE_TOPIC).expect("Invalid topic");
    let message = Message::new(topic, image_data);

    // Publish message periodically
    loop {
        match broker.publish(message.clone()) {
            Ok(_) => println!("Image broadcasted successfully"),
            Err(e) => println!("Failed to broadcast image: {}", e),
        }
        thread::sleep(Duration::from_secs(5));
    }
}

fn run_viewer(save_dir: &str, viewer_id: &str) {
    // Connect to existing broker
    let broker = match Broker::connect(BROKER_NAME) {
        Ok(b) => b,
        Err(e) => {
            println!("Failed to connect to broker: {}", e);
            return;
        }
    };
    println!("Connected to broker");
    // Register as a client
    let client_id = match broker.register_client(&format!("viewer_{}", viewer_id)) {
        Ok(id) => id,
        Err(e) => {
            println!("Failed to register client: {}", e);
            return;
        }
    };

    // Subscribe to image topic
    if let Err(e) = broker.subscribe(&client_id, IMAGE_TOPIC) {
        println!("Failed to subscribe: {}", e);
        return;
    }

    println!(
        "Viewer {} started. Saving images to: {}",
        viewer_id, save_dir
    );

    // Create save directory if it doesn't exist
    let save_path = PathBuf::from(save_dir);
    if let Err(e) = fs::create_dir_all(&save_path) {
        println!("Failed to create save directory: {}", e);
        return;
    }

    // Receive and save images
    let mut counter = 0;
    loop {
        match broker.receive(&client_id) {
            Ok(message) => {
                counter += 1;
                let filename = save_path.join(format!("image_{}_{}.jpg", viewer_id, counter));
                match fs::write(&filename, message.payload) {
                    Ok(_) => println!("Saved image to: {}", filename.display()),
                    Err(e) => println!("Failed to save image: {}", e),
                }
            }
            Err(roker::Error::BufferEmpty) => {
                // Buffer is empty, just wait quietly
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                println!("Error receiving message: {}", e);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}
