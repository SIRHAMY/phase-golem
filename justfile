# Build phase-golem
build:
    cargo build

# Build release
build-release:
    cargo build --release

# Run tests
test:
    cargo test

# Start phase-golem with arguments (e.g., `just start run --target HAMY-001`)
start *ARGS:
    cargo run -- {{ARGS}}

# Run the pipeline targeting a specific backlog item (e.g., `just target HAMY-001`)
target ITEM:
    cargo run -- run --target {{ITEM}}

# Show backlog status
status:
    cargo run -- status

# Triage new backlog items
triage:
    cargo run -- triage
