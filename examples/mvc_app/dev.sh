#!/bin/bash
# Start Tailwind watcher in background
echo "Starting Tailwind watcher..."
npm run watch:css &
TAILWIND_PID=$!

# Start Soli server
echo "Starting Soli server..."
../../target/debug/soli serve .

# Kill Tailwind watcher when Soli server exits
kill $TAILWIND_PID
