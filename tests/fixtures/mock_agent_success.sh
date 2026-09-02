#!/bin/bash
# Mock agent that writes valid result JSON and exits 0
RESULT_PATH="$1"
cat > "$RESULT_PATH" << 'EOF'
{
  "item_id": "018f2b1c-4d5e-7abc-8123-456789abcdef",
  "phase": "prd",
  "result": "phase_complete",
  "summary": "Created PRD with all sections filled",
  "context": null,
  "updated_assessments": null,
  "follow_ups": []
}
EOF
exit 0
