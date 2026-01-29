#!/bin/sh
set -e

# Check for cargo
if ! command -v cargo >/dev/null 2>&1; then
  echo "You muppet, you didn't even install Rust/Cargo and you actually expected it to compile?"
  exit 1
fi

echo "<insert message that tells you that the package is compiling or something>"
cargo build --release

echo "Installing binary..."
sudo install -m 755 target/release/weather-searcher /usr/local/bin/weather-tui

echo "Done. Now enter weather-tui in your terminal of choice to run it."
