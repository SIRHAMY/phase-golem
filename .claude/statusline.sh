#!/bin/bash
input=$(cat)
pct=$(echo "$input" | jq -r '.context_window.used_percentage // 0' | xargs printf "%.0f")

# Start with context info
output="Context: ${pct}%"

# Check if we're in a git repo
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    # Get combined diff stats (staged + unstaged) compared to HEAD
    # Fall back to 'git diff' if HEAD doesn't exist (no commits yet)
    if git rev-parse HEAD >/dev/null 2>&1; then
        stats=$(git diff HEAD --shortstat 2>/dev/null)
    else
        stats=$(git diff --shortstat 2>/dev/null)
    fi

    if [ -n "$stats" ]; then
        # Extract files, insertions, and deletions
        files=$(echo "$stats" | grep -o '[0-9]* file' | awk '{print $1}')
        insertions=$(echo "$stats" | grep -o '[0-9]* insertion' | awk '{print $1}')
        deletions=$(echo "$stats" | grep -o '[0-9]* deletion' | awk '{print $1}')

        # Default to 0 if not found
        files=${files:-0}
        insertions=${insertions:-0}
        deletions=${deletions:-0}

        # Only add if there are changes
        if [ "$files" != "0" ]; then
            output="$output | ${files}F +${insertions}/-${deletions}"
        fi
    fi
fi

printf '%s' "$output"
