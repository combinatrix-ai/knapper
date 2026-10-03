#!/usr/bin/env python3
"""Compare the synthetic bookmark fixture in a running Obsidian app via its CLI."""
import argparse
import json
import pathlib
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--obsidian-cli', required=True)
parser.add_argument('--knapper', required=True)
parser.add_argument('--vault', required=True, type=pathlib.Path)
parser.add_argument('--state', choices=['enabled', 'cold-start-disabled'], required=True)
parser.add_argument('--output', required=True, type=pathlib.Path)
a = parser.parse_args()
vault = a.vault.resolve()

def cli(*args):
    return subprocess.check_output([a.obsidian_cli, 'vault=' + vault.name, *args], text=True, timeout=30).strip()

def evaluate(code):
    raw = cli('eval', 'code=' + code)
    if not raw.startswith('=> '):
        raise RuntimeError(raw)
    return json.loads(raw[3:])

info = evaluate('JSON.stringify({path:app.vault.adapter.getBasePath(),dataview:app.plugins.manifests.dataview?.version,enabled:app.internalPlugins.plugins.bookmarks.enabled})')
if pathlib.Path(info['path']).resolve() != vault:
    raise RuntimeError('CLI connected to the wrong vault')
if info['enabled'] != (a.state == 'enabled'):
    raise RuntimeError('Bookmark plugin state does not match requested state')
for attempt in range(100):
    if evaluate('app.plugins.plugins.dataview.api.index.pages.size') >= 5:
        break
    time.sleep(.1)
else:
    raise RuntimeError('Dataview index did not become ready')
reference = json.loads((pathlib.Path(__file__).resolve().parents[1] / 'tests/fixtures/obsidian-bookmarks/reference-results.json').read_text())
state = next(s for s in reference['states'] if s['lifecycle'] == a.state)
result = {'appVersion': cli('version'), 'dataviewVersion': info['dataview'], 'transport': 'official-obsidian-cli', 'lifecycle': a.state, 'bookmarksCliOutput': cli('bookmarks', 'format=json'), 'comparisons': []}
for row in state['rows']:
    actual = evaluate('(async()=>JSON.stringify(await app.plugins.plugins.dataview.api.query(' + json.dumps(row['query']) + ')))()')
    if not actual['successful']:
        raise RuntimeError(actual)
    actual = actual['value']
    expected = json.loads(subprocess.check_output([a.knapper, '-v', str(vault), '-c', str(vault / 'knapper.yaml'), 'dql', row['query'], '--format', 'json'], text=True, timeout=60))
    normalize = lambda r: {k: r.get(k) for k in ('type', 'headers', 'values')}
    actual, expected = normalize(actual), normalize(expected)
    equal = actual == expected == normalize(row)
    result['comparisons'].append({'query': row['query'], 'obsidian': actual, 'knapper': expected, 'equal': equal})
a.output.write_text(json.dumps(result, indent=2) + '\n')
if not all(r['equal'] for r in result['comparisons']):
    raise SystemExit('Comparison failed; inspect output')
print(f"{a.state}: {len(result['comparisons'])} comparisons passed")
