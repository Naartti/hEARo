"""Print one field from a Claude Code hook's JSON stdin: `_input.py tool_input.file_path`."""

import json
import sys

data = json.load(sys.stdin)
for key in sys.argv[1].split("."):
    data = data.get(key, "") if isinstance(data, dict) else ""
print(data if isinstance(data, str) else json.dumps(data))
