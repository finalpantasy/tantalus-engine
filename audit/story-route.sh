#!/bin/bash
# Story regression: new game (field 70) through the Evil Forest, the Ice
# Cavern, Dali and the airship to the engine room of the cargo ship (field 504), with the
# auto-player. The reference request rules and the real clip lengths are the
# engine defaults; there are no opt-in flags.
#
# usage (from the repo root, tools built with `cargo build --workspace --bins`):
#   bash audit/story-route.sh <out-dir> [path to tantalus-trace]
#
# Step 1 is the chain99 regression itself with `--save-at 301`, which writes
# the world as field 301 starts (the chain output stays byte-identical).
# Step 2 loads that world and walks on with the scripted player input:
#   --hold 307:16        the Ice Cavern ice slide (hold Up)
#   --hold 404:96        the Dali underground ladder (hold Down)
#   --hold 504:16        the cargo ship engine room ladder (hold Up)
#   --skip-talk 455:1,455:2,455:4  the Dali airport talks the route skips
#   --world-exits 9011:14 the world map exit to Dali Village
# The run ends at `--max-fields` 60 (field 504, where the engine room scene
# stalls after the first lines); the expected last state is audit/story-route-expected.json.
set -e
OUT=${1:?out dir}
TRACE=${2:-target/debug/tantalus-trace}
G="${FF9_DIR:-/e/SteamLibrary/steamapps/common/FINAL FANTASY IX}"
M="${MEMORIA_DIR:-/e/FFIX Recomp/.reference/Memoria}"
mkdir -p "$OUT"
COMMON=(--bundle "$G/StreamingAssets/p0data7.bin"
  --event-db "$M/Assembly-CSharp/Global/ff9/FF9DBAll.Events.cs"
  --atan-table "${ATAN_TABLE:-local/probe/ratan_tbl.bin}"
  --character-data "$M/Memoria.Patcher/StreamingAssets/Data/Characters"
  --game-data "$G/x64/FF9_Data"
  --mes-map "$M/Assembly-CSharp/Global/Event/Engine/EventEngineUtils.cs"
  --explore --confirm-after 30 --battles audit/battle-scene-ids.tsv)
CHOICES="52:66=1,64:127=1,202:242=1,202:242=0@2100,202:242=1@2200,652:90=11"
if [ ! -f "$OUT/w301.json" ]; then
  "$TRACE" chain "${COMMON[@]}" --start 70 --ticks 12000 --max-fields 150 \
    --route "100>101,101>102,102>103,103>105,105>106,106>115,153>150,204>205@2250,205>202@2250,202>200@2250" \
    --skip-talk 103:8 --hold 115:16,207:16 --choices "$CHOICES" \
    --waypoints '116@2000:-137/1400:-137/800:-137/-339:1000/-339:1800/-339:2500/1000:5000/2500:8000/3300:10800/3400:11000,154@-807:-5081' \
    --world-exits 9000:4 --save-at 301:"$OUT/w301.json" > "$OUT/part1.jsonl"
fi
"$TRACE" chain "${COMMON[@]}" --start 301 --load-world "$OUT/w301.json" \
  --ticks 12000 --max-fields "${MAX_FIELDS:-60}" --route "301>302" \
  --skip-talk 103:8,455:1,455:2,455:4 --hold 115:16,207:16,307:16,404:96,504:16 \
  --choices "$CHOICES" --world-exits 9011:14 > "$OUT/part2.jsonl"
python -I -X utf8 - "$OUT/part2.jsonl" audit/story-route-expected.json <<'PY'
import json, sys
rows = [json.loads(l) for l in open(sys.argv[1], encoding="utf-8") if l.startswith("{") and '"map"' in l and '"ticks"' in l]
want = json.load(open(sys.argv[2], encoding="utf-8"))
got = {
    "sequence": [r["map"] for r in rows],
    "ticks": [r["ticks"] for r in rows],
    "scenario_counters": [r["scenario_counter"] for r in rows],
}
same = all(got[k] == want[k] for k in got)
print("STORY-ROUTE-SAME" if same else "STORY-ROUTE-DIFFERENT", "last map", rows[-1]["map"], "scenario", rows[-1]["scenario_counter"])
sys.exit(0 if same else 1)
PY
