#!/bin/bash

# Clean up any existing flag files at startup
rm -f /tmp/quorum_updated
debug_mode=true
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
    echo "########################################################"
    echo "~~~~~~~~~~~~~   STARTING NEW ATTESTATION   ~~~~~~~~~~~~~"
    echo "########################################################"
    # Add random operator count selection
    operator_count=$((RANDOM % 3 + 1))
    echo "operator_count for this notorization: $operator_count"
    threshold=$((33 * operator_count))
    signature=$(~/.foundry/bin/cast wallet sign --private-key $PRIVATE_KEY $platform$resource$value$threshold)
    
    # Initialize counter for successful proofs in this batch
    successful_proofs=0
    
    # Outer loop runs until we get enough successful proofs
    while [ $successful_proofs -lt $operator_count ]; do
    echo "########################################################"
    echo "                starting new notarization"
    echo "########################################################"
                # Prepare request data with conditional task_index
        request_data='{
            "address": "'"$address"'",
            "platform": "'"$platform"'",
            "resource": "'"$resource"'",
            "value": "'"$value"'",
            "threshold": '"$threshold"',
            "signature": "'"$signature"'",
            "operator_count": '"$operator_count"'
        }'
        
        # Add task_index to request if we have one
        if [ ! -z "$task_index" ]; then
            request_data=$(echo $request_data | jq '. + {"task_index": '"$task_index"'}')
        fi
        
        node_selector_response=$(curl -X POST -H "Content-Type: application/json" -d "$request_data" "$NODE_SELECTOR")
        
        # Extract node_url and task_index from response
        node_url=$(echo $node_selector_response | jq -r '.node_url')
        new_task_index=$(echo $node_selector_response | jq -r '.task_index')
        
        # Store task_index if this is our first request
        if [ -z "$task_index" ]; then
            task_index=$new_task_index
        fi
        
        echo "Task index: $task_index"
        echo "node_url: $node_url"
        echo "node_selector_response: $node_selector_response"
        if [ -z "$node_url" ] || [ "$node_url" == "null" ]; then
            echo "Failed to get a valid node_url. Retrying in 5 seconds..."
            sleep 5
            continue
        fi
            
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
               echo "$response" | grep -q "Failed to verify against notary public key" || \
               echo "$response" | grep -q "Resource value .* does not match expected" || \
               echo "$response" | grep -q "Resource .* not found in response" || \
               echo "$response" | grep -q "Invalid HTTP response format" || \
               echo "$response" | grep -q "Failed to parse JSON response" || \
               echo "$response" | grep -q "No 'result' object found in response" || \
               echo "$response" | grep -q "Resource value is not a string" || \
               echo "$response" | grep -q "Server name does not match platform" || \
               echo "$response" | grep -q "Commitment timestamp is too old" || \
               echo "$response" | grep -q "Invalid commitment signature" || \
               echo "$response" | grep -q "Invalid node selector signature"; then
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
            successful_proofs=$((successful_proofs + 1))
            echo "Successful proof $successful_proofs of $operator_count"

            
            # Process successful aggregation response
                else 
                    echo "Request failed"
                fi
            done
            # Check for aggregator errors
            if [ -z "$aggregator_response" ] || \
               [ "$aggregator_response" = "{}" ] || \
               echo "$aggregator_response" | grep -q "Failed to parse outer JSON" || \
               echo "$aggregator_response" | grep -q "Failed to get inner JSON string" || \
               echo "$aggregator_response" | grep -q "Failed to parse inner JSON" || \
               echo "$aggregator_response" | grep -q "Failed to process signature" || \
               echo "$aggregator_response" | grep -q "Error in aggregation response" || \
               echo "$aggregator_response" | grep -q "Timeout waiting for aggregation response" || \
               echo "$aggregator_response" | grep -q "Aggregation channel closed without response" || \
               echo "$aggregator_response" | grep -q "No signature field found in JSON" || \
               echo "$aggregator_response" | grep -q "No operator_address field found in JSON" || \
               echo "$aggregator_response" | grep -q "No operator_id field found in JSON" || \
               echo "$aggregator_response" | grep -q "No commitment_hash field found in JSON" || \
               echo "$aggregator_response" | grep -q "No task_index field found in JSON"; then
                echo "Aggregation failed with error: $aggregator_response"
                sleep 5
                continue
            fi

            # Debug: Print aggregator response
            if [ "$debug_mode" = true ]; then
                echo "Aggregator Response: $aggregator_response"
            fi

            # Extract values from aggregator response more safely
            NON_SIGNER_COUNT=$(echo $aggregator_response | jq -r '.non_signer_bitmap_indices | length')
            echo "Number of non-signers: $NON_SIGNER_COUNT"

            # Build the non-signer arrays dynamically
            BITMAP_INDICES_ARR=$(echo $aggregator_response | jq -r '.non_signer_bitmap_indices | join(",")')
            BITMAP_INDICES_ARR=${BITMAP_INDICES_ARR:-""}

            # Build the public keys arrays
            PUBLIC_KEYS_ARR=""
            for i in $(seq 0 $(($NON_SIGNER_COUNT - 1))); do
                if [ $i -gt 0 ]; then
                    PUBLIC_KEYS_ARR="$PUBLIC_KEYS_ARR,"
                fi
                x=$(echo $aggregator_response | jq -r ".non_signer_public_keys[$i].x")
                y=$(echo $aggregator_response | jq -r ".non_signer_public_keys[$i].y")
                PUBLIC_KEYS_ARR="$PUBLIC_KEYS_ARR($x,$y)"
            done

            # Build the stake indices array
            STAKE_INDICES_ARR=$(echo $aggregator_response | jq -r '.non_signer_stake_indices[0] | join(",")')
            STAKE_INDICES_ARR=${STAKE_INDICES_ARR:-""}

            # Extract other required values
            QUORUM_APK_INDICES=$(echo $aggregator_response | jq -r '.quorum_apk_indices[0]')
            TOTAL_STAKE_INDICES=$(echo $aggregator_response | jq -r '.total_stake_indices[0]')
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
                    cast_command="~/.foundry/bin/cast send $REGISTRY_COORDINATOR_ADDRESS --private-key $PRIVATE_KEY \"updateOperatorsForQuorum(address[][],bytes)\" [[${operator_address_list}]] 0x00 --rpc-url $RPC_URL"                    
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

            echo 'cast send $BLS_SIGNATURE_CHECKER_ADDRESS "checkSignatures(bytes32,bytes,uint32,(uint32[],(uint256,uint256)[],(uint256,uint256)[],(uint256[2],uint256[2]),(uint256,uint256),uint32[],uint32[],uint32[][]))" $MSG_HASH $QUORUM_NUMBERS $REF_BLOCK_NUMBER "([$BITMAP_INDICES_ARR],[$PUBLIC_KEYS_ARR],[($APK_G1_X,$APK_G1_Y)],([$APK_G2_X1,$APK_G2_X2],[$APK_G2_Y1,$APK_G2_Y2]),($SIG_G1_X,$SIG_G1_Y),[$QUORUM_APK_INDICES],[$TOTAL_STAKE_INDICES],[[$STAKE_INDICES_ARR]])" --rpc-url $RPC_URL --private-key $PRIVATE_KEY) '

            sig_verification=$(~/.foundry/bin/cast send $BLS_SIGNATURE_CHECKER_ADDRESS \
            "checkSignatures(bytes32,bytes,uint32,(uint32[],(uint256,uint256)[],(uint256,uint256)[],(uint256[2],uint256[2]),(uint256,uint256),uint32[],uint32[],uint32[][]))" \
            $MSG_HASH \
            $QUORUM_NUMBERS \
            $REF_BLOCK_NUMBER \
            "([$BITMAP_INDICES_ARR],\
            [$PUBLIC_KEYS_ARR],\
            [($APK_G1_X,$APK_G1_Y)],\
            ([$APK_G2_X1,$APK_G2_X2],[$APK_G2_Y1,$APK_G2_Y2]),\
            ($SIG_G1_X,$SIG_G1_Y),\
            [$QUORUM_APK_INDICES],\
            [$TOTAL_STAKE_INDICES],\
            [[$STAKE_INDICES_ARR]])" \
            --rpc-url $RPC_URL \
            --private-key $PRIVATE_KEY)
            echo "Signature Verification: $sig_verification"
            task_index=""
done 
