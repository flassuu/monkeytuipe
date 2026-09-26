#!/usr/bin/env bash
# Refresh the word lists bundled into the binary.
#
# The full upstream catalogue is ~140 MB, so only the base (200-word) list of a
# handful of common languages is embedded; everything else is downloaded on
# demand and cached. Run this when upstream changes a list you care about, and
# commit the result.
set -euo pipefail

UPSTREAM="https://raw.githubusercontent.com/monkeytypegame/monkeytype/master/frontend/static/languages"
here="$(cd "$(dirname "$0")/.." && pwd)"
dest="$here/src/words/data"
mkdir -p "$dest"

# Keep this list in sync with `EMBEDDED` in src/words/language.rs.
languages=(english russian german spanish french portuguese)

for lang in "${languages[@]}"; do
    echo "fetching $lang"
    curl -fsS "$UPSTREAM/$lang.json" -o "$dest/$lang.json"
done

echo
echo "embedded total: $(du -ch -- "$dest"/*.json | tail -1 | cut -f1)"
echo "per language:"
for lang in "${languages[@]}"; do
    printf '  %-14s %7s bytes  %4s words\n' \
        "$lang" \
        "$(wc -c <"$dest/$lang.json")" \
        "$(python3 -c "import json,sys;print(len(json.load(open(sys.argv[1]))['words']))" "$dest/$lang.json")"
done
