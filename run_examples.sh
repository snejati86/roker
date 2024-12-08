#!/bin/bash

# Enable debug mode
set -x

# Create necessary directories
mkdir -p ./test_images
mkdir -p ./received_images

# Create a test image if it doesn't exist (1024x768 black image)
if [ ! -f "./test_images/test.jpg" ]; then
    echo "Creating test image..."
    convert -size 1024x768 xc:black ./test_images/test.jpg
fi

# Function to cleanup processes
cleanup() {
    echo "=== Cleanup triggered ==="
    if [ -f publisher.pid ]; then
        echo "Killing publisher process: $(cat publisher.pid)"
        kill $(cat publisher.pid) 2>/dev/null
        rm publisher.pid
    fi
    if [ -f viewer.pid ]; then
        echo "Killing viewer process: $(cat viewer.pid)"
        kill $(cat viewer.pid) 2>/dev/null
        rm viewer.pid
    fi
    
    # Print final logs
    echo "=== Final Publisher Log ==="
    cat publisher.log
    echo "=== Final Viewer Log ==="
    cat viewer.log
    
    exit 0
}

# Set up cleanup on script exit
trap cleanup EXIT INT TERM

# Start the publisher in the background
echo "=== Starting publisher ==="
RUST_LOG=debug cargo run --example image_broadcast publisher ./test_images/test.jpg > publisher.log 2>&1 &
publisher_pid=$!
echo $publisher_pid > publisher.pid

# Wait for publisher to initialize (look for successful startup message)
max_attempts=10
attempt=0
while [ $attempt -lt $max_attempts ]; do
    echo "Waiting for publisher to initialize (attempt $((attempt + 1))/$max_attempts)"
    if grep -q "Publisher started" publisher.log; then
        echo "Publisher successfully initialized"
        # Show the last few lines of publisher log
        echo "=== Publisher Log ==="
        tail -n 5 publisher.log
        break
    fi
    sleep 1
    attempt=$((attempt + 1))
done

if [ $attempt -eq $max_attempts ]; then
    echo "Failed to start publisher"
    echo "=== Publisher Log ==="
    cat publisher.log
    cleanup
    exit 1
fi

# Give the broker a moment to fully initialize
sleep 2

# Start the viewer in the background
echo "=== Starting viewer ==="
RUST_LOG=debug cargo run --example image_broadcast viewer ./received_images viewer1 > viewer.log 2>&1 &
viewer_pid=$!
echo $viewer_pid > viewer.pid

# Check if viewer connects successfully
attempt=0
while [ $attempt -lt $max_attempts ]; do
    echo "Waiting for viewer to connect (attempt $((attempt + 1))/$max_attempts)"
    if grep -q "Viewer.*started" viewer.log; then
        echo "Viewer successfully connected"
        echo "=== Viewer Log ==="
        tail -n 5 viewer.log
        break
    elif grep -q "Failed to connect" viewer.log; then
        echo "Viewer failed to connect to broker"
        echo "=== Viewer Log ==="
        cat viewer.log
        cleanup
        exit 1
    fi
    sleep 1
    attempt=$((attempt + 1))
done

echo "Both processes started. Press Ctrl+C to stop."
echo "Images will be saved in ./received_images/"
echo "Check publisher.log and viewer.log for detailed output"

# Monitor both processes and their logs
while kill -0 $publisher_pid 2>/dev/null && kill -0 $viewer_pid 2>/dev/null; do
    echo "=== Recent Publisher Log ==="
    tail -n 2 publisher.log
    echo "=== Recent Viewer Log ==="
    tail -n 2 viewer.log
    sleep 5
done

# If we get here, one of the processes died
echo "One of the processes exited unexpectedly"
echo "=== Final Publisher Log ==="
cat publisher.log
echo "=== Final Viewer Log ==="
cat viewer.log
cleanup