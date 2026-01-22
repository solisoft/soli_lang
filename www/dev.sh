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

# Start Soli server in dev mode
# Tailwind CSS compilation is now integrated - no need for separate watcher!
echo -e "${GREEN}Starting Soli server with hot reload...${NC}"
echo -e "${GREEN}Tailwind CSS will be compiled automatically when views change.${NC}"
soli serve . --dev
