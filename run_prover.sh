#!/bin/bash

# Clean up any existing flag files at startup
rm -f /tmp/quorum_updated
debug_mode=false
counter=0
address=$(~/.foundry/bin/cast wallet address $PRIVATE_KEY)
platform=api.cloudflare.com
resource=model
value=gpt-4o-mini
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

            # Check for verification errors
            if echo "$response" | grep -q "Error verifying session proof" || \
               echo "$response" | grep -q "Error verifying substrings proof" || \
               echo "$response" | grep -q "Failed to parse proof" || \
               echo "$response" | grep -q "Failed to verify against notary public key"; then
                echo "Verification failed with error: $response"
                sleep 5
                continue
            fi

            # Debug: Print verify response
            if [ "$debug_mode" = true ]; then
                echo "Verify Response: $response"
            fi

            # Send to aggregator and capture its response
            aggregator_response=$(curl -X POST -d "$response" http://127.0.0.1:5074/aggregate)

            # Debug: Print aggregator response
            if [ "$debug_mode" = true ]; then
                echo "Aggregator Response: $aggregator_response"
            fi

            # Check for aggregator errors
            if [ -z "$aggregator_response" ] || \
               [ "$aggregator_response" = "{}" ] || \
               echo "$aggregator_response" | grep -q "Failed to process signature" || \
               echo "$aggregator_response" | grep -q "Error in aggregation response" || \
               echo "$aggregator_response" | grep -q "Timeout waiting for aggregation response" || \
               echo "$aggregator_response" | grep -q "Aggregation channel closed without response"; then
                echo "Aggregation failed with error: $aggregator_response"
                sleep 5
                continue
            fi

            # Extract values from aggregator response
            NON_SIGNER_BITMAP_INDICES_0=$(echo $aggregator_response | jq -r '.non_signer_bitmap_indices[0]')
            NON_SIGNER_BITMAP_INDICES_1=$(echo $aggregator_response | jq -r '.non_signer_bitmap_indices[1]')
            NON_SIGNER_PUBLIC_KEYS_0_X=$(echo $aggregator_response | jq -r '.non_signer_public_keys[0].x')
            NON_SIGNER_PUBLIC_KEYS_0_Y=$(echo $aggregator_response | jq -r '.non_signer_public_keys[0].y')
            NON_SIGNER_PUBLIC_KEYS_1_X=$(echo $aggregator_response | jq -r '.non_signer_public_keys[1].x')
            NON_SIGNER_PUBLIC_KEYS_1_Y=$(echo $aggregator_response | jq -r '.non_signer_public_keys[1].y')
            QUORUM_APK_INDICES=$(echo $aggregator_response | jq -r '.quorum_apk_indices[0]')
            TOTAL_STAKE_INDICES=$(echo $aggregator_response | jq -r '.total_stake_indices[0]')
            NON_SIGNER_STAKE_INDICES_0=$(echo $aggregator_response | jq -r '.non_signer_stake_indices[0][0]')
            NON_SIGNER_STAKE_INDICES_1=$(echo $aggregator_response | jq -r '.non_signer_stake_indices[0][1]')
            SIG_G1_X=$(echo $aggregator_response | jq -r '.sig_g1_x')
            SIG_G1_Y=$(echo $aggregator_response | jq -r '.sig_g1_y')
            APK_G1_X=$(echo $aggregator_response | jq -r '.apk_g1_x')
            APK_G1_Y=$(echo $aggregator_response | jq -r '.apk_g1_y')
            APK_G2_X1=$(echo $aggregator_response | jq -r '.apk_g2_x1')
            APK_G2_X2=$(echo $aggregator_response | jq -r '.apk_g2_x2')
            APK_G2_Y1=$(echo $aggregator_response | jq -r '.apk_g2_y1')
            APK_G2_Y2=$(echo $aggregator_response | jq -r '.apk_g2_y2')
            MSG_HASH=$(echo $aggregator_response | jq -r '.task_response_digest')
            QUORUM_NUMBERS="0x00"

            # Get current block and set reference block
            CURRENT_BLOCK=$(~/.foundry/bin/cast block latest --rpc-url http://ethereum:8545 | grep "number" | awk '{print $2}')
            REF_BLOCK_NUMBER=$((CURRENT_BLOCK-1))
            
            # Check if we've already updated quorum (using a flag file in /tmp)
            if [ ! -f "/tmp/quorum_updated" ]; then
                echo "Updating quorum..."
               
               # Debug: List all files in operator_keys directory
                if [ "$debug_mode" = true ]; then
                    echo "Contents of /app/operator_keys:"
                    ls -la /app/operator_keys
                fi
                
                # Get all operator addresses from key files and sort them hexadecimally
                operator_addresses=$(find "/app/operator_keys" -name "testacc*.ecdsa.key.json" -exec jq -r '"0x" + .address' {} \; | sort -k1.3)
                
                # Debug: Show found addresses
                if [ "$debug_mode" = true ]; then
                    echo "Found operator addresses:"
                    echo "$operator_addresses"
                fi
                
                # Convert newline-separated addresses into comma-separated list for cast command
                operator_address_list=$(echo "$operator_addresses" | tr '\n' ',' | sed 's/,$//')
                
                if [ "$debug_mode" = true ]; then
                    echo "Operator address list: $operator_address_list"
                fi
                
                if [ -n "$operator_address_list" ]; then
                    # Construct and execute the cast command
                    cast_command="~/.foundry/bin/cast send 0xeCd099fA5048c3738a5544347D8cBc8076E76494 --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \"updateOperatorsForQuorum(address[][],bytes)\" [[${operator_address_list}]] 0x00 --rpc-url http://ethereum:8545"
                    
                    echo "Executing cast command..."
                    eval "$cast_command"
                    
                    # Create flag file in /tmp to indicate quorum has been updated
                    touch "/tmp/quorum_updated"
                    echo "Quorum updated successfully"
                else
                    echo "No operator addresses found!"
                fi
            else
                echo "Quorum already updated (flag file exists)"
            fi

            # Execute checkSignatures call
            echo "verifying signature onchain..."

            sig_verification=$(~/.foundry/bin/cast call $BLS_SIGNATURE_CHECKER_ADDRESS \
            "checkSignatures(bytes32,bytes,uint32,(uint32[],(uint256,uint256)[],(uint256,uint256)[],(uint256[2],uint256[2]),(uint256,uint256),uint32[],uint32[],uint32[][]))" \
            $MSG_HASH \
            $QUORUM_NUMBERS \
            $REF_BLOCK_NUMBER \
            "([$NON_SIGNER_BITMAP_INDICES_0,$NON_SIGNER_BITMAP_INDICES_1],\
            [($NON_SIGNER_PUBLIC_KEYS_0_X,$NON_SIGNER_PUBLIC_KEYS_0_Y),($NON_SIGNER_PUBLIC_KEYS_1_X,$NON_SIGNER_PUBLIC_KEYS_1_Y)],\
            [($APK_G1_X,$APK_G1_Y)],\
            ([$APK_G2_X1,$APK_G2_X2],[$APK_G2_Y1,$APK_G2_Y2]),\
            ($SIG_G1_X,$SIG_G1_Y),\
            [$QUORUM_APK_INDICES],\
            [$TOTAL_STAKE_INDICES],\
            [[$NON_SIGNER_STAKE_INDICES_0,$NON_SIGNER_STAKE_INDICES_1]])" \
            --rpc-url http://ethereum:8545)
            echo "Signature Verification: $sig_verification"

        else 
            echo "Request failed"
        fi
    fi
    sleep 5
done 
