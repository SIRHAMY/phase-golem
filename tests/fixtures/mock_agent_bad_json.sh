#!/bin/bash
# Mock agent that writes invalid JSON to result file
RESULT_PATH="$1"
echo '{"item_id": "WRK-001", "phase": "not valid json...' > "$RESULT_PATH"
exit 0
