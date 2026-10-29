# Self-check of the grading in agents.py, no agent runs: python3 bench/test_agents.py
import agents

task = {"id": "t", "prompt": "p", "expect": ["nature/x.jpg"]}
lines = [
    '{"type":"system","subtype":"init","cwd":"/tmp/agent-bench-1","mcp_servers":[{"name":"luma","status":"connected"}]}',
    '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__luma__luma_search_images","input":{"query":"x"}},{"type":"tool_use","name":"Read","input":{}}]}}',
    '{"type":"result","is_error":false,"result":"Нашёл: ~/Photos/nature/x.jpg","num_turns":3,"duration_ms":4200,"total_cost_usd":0.05}',
]
r = agents.grade(task, "mcp", agents.parse_stream(lines), "", 5)
assert r["found"] and r["luma_calls"] == 1 and r["reads"] == 1 and r["setup_ok"] and r["error"] is None, r
assert r["seconds"] == 4.2, r

# `luma query` through Bash counts as a Luma call; Luma visible to the baseline = broken setup.
lines[1] = '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"luma query clip x"}}]}}'
r = agents.grade(task, "none", agents.parse_stream(lines), "", 5)
assert r["luma_calls"] == 1 and not r["setup_ok"], r

# Timeout: no result message at all.
r = agents.grade(task, "cli", ({}, {}, []), "timeout", 600)
assert r["error"] == "timeout" and not r["found"] and r["seconds"] == 600, r

# Control task: nothing expected, so never "found"; calling Luma on it is flagged.
r = agents.grade({"id": "c", "prompt": "p", "expect": []}, "none", agents.parse_stream(lines), "", 5)
assert r["control"] and not r["found"], r
assert agents.outcome(r) == "ERR"  # baseline that saw Luma: setup broken wins
r = agents.grade({"id": "c", "prompt": "p", "expect": []}, "cli", agents.parse_stream(lines), "", 5)
assert agents.outcome(r) == "LUMA", r

# The report renders every outcome and names the problem runs.
ok = agents.grade(task, "mcp", agents.parse_stream([lines[0], lines[2]]), "", 5)
rows = [ok, r, agents.grade(task, "mcp", ({}, {}, []), "timeout", 600)]
text = agents.report([row | {"model": "sonnet", "effort": "medium"} for row in rows], color=False)
assert "sonnet / medium" in text and "✗ LUMA" in text and "✗ ERR" in text and "timeout" in text, text

print("ok")
