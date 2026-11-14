"""Read-only audit of the most recent isolated native chat acceptance database."""
import collections
import json
import pathlib
import re
import sqlite3

root = pathlib.Path(__file__).resolve().parents[1]
output = root / "output/playwright/chat-lifecycle"
run = json.loads((output / "run-info.json").read_text(encoding="utf-8-sig"))
database = pathlib.Path(run["dataRoot"]) / "com.chyin.kokoro/kokoro.db"
connection = sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)
connection.row_factory = sqlite3.Row
rows = [dict(row) for row in connection.execute("""
    SELECT m.id, m.conversation_id, c.character_id, m.role, m.content, m.metadata
    FROM conversation_messages m JOIN conversations c ON c.id = m.conversation_id ORDER BY m.id
""")]
case_counts = collections.Counter()
for row in rows:
    case = re.search(r"\[case:([^\]]+)\]", row["content"])
    if case:
        case_counts[(case.group(1), row["role"], row["character_id"])] += 1
report = {
    "database": str(database),
    "integrity": [row[0] for row in connection.execute("PRAGMA integrity_check")],
    "foreignKeyViolations": [list(row) for row in connection.execute("PRAGMA foreign_key_check")],
    "cases": [{"case": case, "role": role, "characterId": character, "count": count}
              for (case, role, character), count in sorted(case_counts.items())],
    "messages": rows,
}
connection.close()
assert report["integrity"] == ["ok"]
assert not report["foreignKeyViolations"]
for name in ["history-regenerated", "main-hidden", "sender-destroyed-slow", "main-destroyed"]:
    for role in ["user", "assistant"]:
        assert case_counts[(name, role, "chat-validation-character")] == 1, (name, role)
assert case_counts[("other-character", "user", "chat-validation-other")] == 1
assert case_counts[("other-character", "assistant", "chat-validation-other")] == 1
(output / "database-audit.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
print(json.dumps({"integrity": report["integrity"], "foreignKeyViolations": report["foreignKeyViolations"], "caseGroups": len(report["cases"])}))
