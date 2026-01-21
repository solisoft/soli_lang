#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}Starting Soli MVC Development Server...${NC}"

# Check if npm dependencies are installed
if [ ! -d "node_modules" ]; then
    echo -e "${YELLOW}Installing npm dependencies...${NC}"
    npm install
fi

# Build CSS first
echo -e "${GREEN}Building CSS...${NC}"
npm run build:css

# Start Tailwind watcher in background
echo -e "${GREEN}Starting Tailwind watcher...${NC}"
npm run watch:css &
TAILWIND_PID=$!

# Trap signals to clean up
cleanup() {
    echo -e "${YELLOW}Shutting down...${NC}"
    kill $TAILWIND_PID 2>/dev/null || true
    exit 0
}
trap cleanup SIGINT SIGTERM

# Start Soli server
echo -e "${GREEN}Starting Soli server...${NC}"
soli serve .

# Kill Tailwind watcher when Soli server exits
kill $TAILWIND_PID
