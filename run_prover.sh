#!/bin/bash

counter=0
address=$(~/.foundry/bin/cast wallet address $PRIVATE_KEY)
platform=kalshi
resource=open_time
value=10:00:00UTC
threshold=1 
signature=$(~/.foundry/bin/cast wallet sign --private-key $PRIVATE_KEY $platform$resource$value$threshold)
echo "starting aggregator"
/usr/bin/aggregator &
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
    task_index=$(echo $node_selector_response | jq -r '.task_index')
    echo "Task index: $task_index"
        
    if [ -z "$node_url" ] || [ "$node_url" == "null" ]; then
        echo "Failed to get a valid node_url. Retrying in 5 seconds..."
    else
        
        node_url=${node_url#http://}
        echo "Running prover with node_url: $node_url"
        /usr/bin/prover $node_url 7047
        
        if [ $? -eq 0 ]; then
            counter=$((counter + 1))
            mv simple_proof.json proof_$counter.json
            echo "Proof saved as proof_$counter.json"
            echo "Creating combined proof json..."
            
            # Create combined json with all fields
            jq -n \
            --arg address "$address" \
            --arg operator_id "$operator_id" \
            --arg platform "$platform" \
            --arg resource "$resource" \
            --arg value "$value" \
            --argjson threshold "$threshold" \
            --arg signature "$signature" \
            --argjson task_index "$task_index" \
            --argjson node_selector_response "$node_selector_response" \
            --slurpfile tls_proof "proof_$counter.json" \
            '{
                address: $address,
                operator_id: $operator_id,
                platform: $platform,
                resource: $resource,
                value: $value,
                threshold: $threshold,
                signature: $signature,
                node_url: $node_selector_response.node_url,
                timestamp: $node_selector_response.timestamp,
                node_selector_signature: $node_selector_response.node_selector_signature,
                tls_proof: $tls_proof[0],
                task_index: $task_index
            }' > combined_proof_$counter.json
            
            echo "Combined proof saved as combined_proof_$counter.json"
            echo "Submitting combined proof for verification..."
            response=$(curl -X POST -H "Content-Type: application/json" -d @combined_proof_$counter.json "$node_url:6074/verify")
            echo "Verify Response: $response"

            # Send to aggregator and capture its response
            aggregator_response=$(curl -X POST -d "$response" http://127.0.0.1:5074/aggregate)
            echo "Aggregator Response: $aggregator_response"

            # Parse the BlsAggregationServiceResponse
            task_index=$(echo $aggregator_response | jq -r '.task_index')
            task_response_digest=$(echo $aggregator_response | jq -r '.task_response_digest')
            signers_agg_sig=$(echo $aggregator_response | jq -r '.signers_agg_sig_g1.g1_point')
            signers_apk_g2=$(echo $aggregator_response | jq -r '.signers_apk_g2')
            quorum_apks_g1=$(echo $aggregator_response | jq -r '.quorum_apks_g1[0]')

            # Extract coordinates from the arrays
            SIG_G1_X=$(echo $signers_agg_sig | jq -r '.[0:32] | reduce .[] as $num (0; . * 256 + $num)')
            SIG_G1_Y=$(echo $signers_agg_sig | jq -r '.[32:64] | reduce .[] as $num (0; . * 256 + $num)')
            
            APK_G1_X=$(echo $quorum_apks_g1 | jq -r '.[0:32] | reduce .[] as $num (0; . * 256 + $num)')
            APK_G1_Y=$(echo $quorum_apks_g1 | jq -r '.[32:64] | reduce .[] as $num (0; . * 256 + $num)')
            
            APK_G2_X1=$(echo $signers_apk_g2 | jq -r '.[0:32] | reduce .[] as $num (0; . * 256 + $num)')
            APK_G2_X2=$(echo $signers_apk_g2 | jq -r '.[32:64] | reduce .[] as $num (0; . * 256 + $num)')
            APK_G2_Y1=$(echo $signers_apk_g2 | jq -r '.[64:96] | reduce .[] as $num (0; . * 256 + $num)')
            APK_G2_Y2=$(echo $signers_apk_g2 | jq -r '.[96:128] | reduce .[] as $num (0; . * 256 + $num)')

            MSG_HASH=$task_response_digest

            echo "Extracted values for cast call:"
            echo "MSG_HASH: $MSG_HASH"
            echo "APK_G1: ($APK_G1_X,$APK_G1_Y)"
            echo "APK_G2: ([$APK_G2_X2,$APK_G2_X1],[$APK_G2_Y2,$APK_G2_Y1])"
            echo "SIG_G1: ($SIG_G1_X,$SIG_G1_Y)"

            # Execute cast call
            sig_verification=$(~/.foundry/bin/cast call 0xb7ba8bbc36AA5684fC44D02aD666dF8E23BEEbF8 --rpc-url http://ethereum:8545 \
                "trySignatureAndApkVerification(bytes32,(uint256,uint256),(uint256[2],uint256[2]),(uint256,uint256))(bool,bool)" \
                $MSG_HASH \
                "($APK_G1_X,$APK_G1_Y)" \
                "([$APK_G2_X2,$APK_G2_X1],[$APK_G2_Y2,$APK_G2_Y1])" \
                "($SIG_G1_X,$SIG_G1_Y)")
            echo "Signature Verification: $sig_verification"

        else 
            echo "Request failed"
        fi
    fi
    sleep 5
done 
