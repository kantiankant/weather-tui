#!/bin/zsh
echo "Building weather-searcher..."
cargo build --release

echo "Installing to /usr/local/bin..."
sudo cp target/release/weather-searcher /usr/local/bin/weather-searcher

echo "✓ Updated successfully!"
echo "Run 'weather-searcher' to test"
