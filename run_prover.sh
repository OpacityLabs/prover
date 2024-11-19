#!/bin/bash

counter=0
address=$(cast wallet address $PRIVATE_KEY)
platform=kalshi
resource=open_time
value=10:00:00UTC
threshold=1 
signature=$(cast wallet sign --private-key $PRIVATE_KEY $platform$resource$value$threshold)


while true; do
    node_url=$(curl -X POST -H "Content-Type: application/json" -d '{"address":"'"$address"'","platform":"'"$platform"'","resource":"'"$resource"'","value":"'"$value"'","threshold":'"$threshold"',"signature":"'"$signature"'"}' $NODE_SELECTOR | jq -r '.node_url')
    if [ -z "$node_url" ] || [ "$node_url" == "null" ]; then
        echo "Failed to get a valid node_url. Retrying in 5 seconds..."
    else
        
        node_url=${node_url#http://}
        echo "Running prover with node_url: $node_url"
        ./target/release/prover $node_url 7047
        
        if [ $? -eq 0 ]; then
            counter=$((counter + 1))
            mv simple_proof.json proof_$counter.json
            echo "Proof saved as proof_$counter.json"
            echo "Submitting proof for verification..."
            curl -X POST -H "Content-Type: application/json" -d @proof_$counter.json $node_url:6074/verify
        else 
            echo "Request failed"
        fi
    fi
    sleep 5
done
