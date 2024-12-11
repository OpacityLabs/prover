#!/bin/bash

counter=0
address=$(~/.foundry/bin/cast wallet address $PRIVATE_KEY)
platform=kalshi
resource=open_time
value=10:00:00UTC
threshold=1 
signature=$(~/.foundry/bin/cast wallet sign --private-key $PRIVATE_KEY $platform$resource$value$threshold)
echo "starting aggregator"
./target/release/aggregator &
echo "starting prover"
while true; do
    node_selector_response=$(curl -X POST -H "Content-Type: application/json" -d '{
        "address": "'"$address"'",
        "platform": "'"$platform"'",
        "resource": "'"$resource"'",
        "value": "'"$value"'",
        "threshold": '"$threshold"',
        "signature": "'"$signature"'"
    }' "$NODE_SELECTOR")
    node_url=$(echo $node_selector_response | jq -r '.node_url')
        
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
            echo "Creating combined proof json..."
            
            # Create combined json with all fields
            jq -n \
            --arg address "$address" \
            --arg platform "$platform" \
            --arg resource "$resource" \
            --arg value "$value" \
            --argjson threshold "$threshold" \
            --arg signature "$signature" \
            --argjson node_selector_response "$node_selector_response" \
            --slurpfile tls_proof "proof_$counter.json" \
            '{
                address: $address,
                platform: $platform,
                resource: $resource,
                value: $value,
                threshold: $threshold,
                signature: $signature,
                node_url: $node_selector_response.node_url,
                timestamp: $node_selector_response.timestamp,
                node_selector_signature: $node_selector_response.node_selector_signature,
                tls_proof: $tls_proof[0]
            }' > combined_proof_$counter.json
            
            echo "Combined proof saved as combined_proof_$counter.json"
            echo "Submitting combined proof for verification..."
            response=$(curl -X POST -H "Content-Type: application/json" -d @combined_proof_$counter.json $node_url:6074/verify)
            echo "Response: $response"
            curl -X POST -d "$response"  http://127.0.0.1:5074/aggregate
        else 
            echo "Request failed"
        fi
    fi
    sleep 5
done 