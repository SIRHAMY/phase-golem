#!/bin/bash
# Mock agent that writes invalid JSON to result file
RESULT_PATH="$1"
echo '{"item_id": "018f2b1c-4d5e-7abc-8123-456789abcdef", "phase": "not valid json...' > "$RESULT_PATH"
exit 0
