FROM rust 
RUN apt-get update && apt-get install -y curl jq 
RUN curl -L https://foundry.paradigm.xyz | bash
RUN ~/.foundry/bin/foundryup 

WORKDIR /opacity-simple-prover
COPY . .
RUN cargo build --release
RUN mv target/release/prover /usr/bin/prover
RUN mv target/release/aggregator /usr/bin/aggregator
COPY run_prover.sh /opacity-simple-prover/run_prover.sh
RUN chmod +x /opacity-simple-prover/run_prover.sh
ENTRYPOINT ["/opacity-simple-prover/run_prover.sh"]