#!/usr/bin/env python3
"""Run the extensible PKM cases, optionally against a live Obsidian App."""
import argparse
import datetime
import json
import os
import pathlib
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--knapper', required=True)
    parser.add_argument('--vault', required=True, type=pathlib.Path)
    parser.add_argument('--obsidian-cli')
    parser.add_argument('--output', required=True, type=pathlib.Path)
    parser.add_argument('--watch', action='store_true', help='Poll App/knapper parity after edits; do not require baseline values')
    parser.add_argument('--interval', type=float, default=5)
    parser.add_argument('--iterations', type=int, default=0, help='Stop watch after N passes (0: until interrupted)')
    args = parser.parse_args()
    if args.interval <= 0 or args.iterations < 0:
        parser.error('interval must be positive; iterations must be nonnegative')
    if args.watch and not args.obsidian_cli:
        parser.error('--watch requires --obsidian-cli')
    vault = args.vault.resolve()
    manifest = json.loads((vault / 'cases.json').read_text())
    if manifest['schemaVersion'] != 1:
        parser.error('unsupported cases schema')

    def command(argv):
        return subprocess.check_output(argv, text=True, timeout=60, env={**os.environ, 'KNAPPER_NO_UPDATE_CHECK': '1'}).strip()

    def cli(*argv):
        return command([args.obsidian_cli, 'vault=' + vault.name, *argv])

    def evaluate(code):
        raw = cli('eval', 'code=' + code)
        if not raw.startswith('=> '):
            raise RuntimeError(raw)
        return json.loads(raw[3:])

    def normalize(value, case):
        if value['type'] == 'task':
            tasks = value.get('tasks')
            if tasks is None:
                tasks = value['values']
            return {'type': 'task', 'tasks': [{k: t.get(k) for k in case['taskFields']} for t in tasks]}
        return {k: value.get(k) for k in case['expected']}

    iteration = 0
    while True:
        result = {'verifiedAt': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'mode': 'live-parity' if args.watch else 'baseline', 'comparisons': []}
        if args.obsidian_cli:
            info = evaluate('JSON.stringify({path:app.vault.adapter.getBasePath(),dataview:app.plugins.manifests.dataview?.version,pages:app.plugins.plugins.dataview?.api?.index?.pages?.size,timezone:Intl.DateTimeFormat().resolvedOptions().timeZone})')
            if pathlib.Path(info['path']).resolve() != vault:
                raise RuntimeError('CLI connected to the wrong vault')
            if info['timezone'] != manifest['timezone']:
                raise RuntimeError('Obsidian timezone differs from fixture timezone')
            if not info['pages']:
                raise RuntimeError('Dataview index is not ready')
            result.update(appVersion=cli('version'), dataviewVersion=info['dataview'])
        for case in manifest['cases']:
            # Retry live disagreements during App's asynchronous reindexing;
            # persistent mismatches still fail and are saved verbatim.
            for attempt in range(10 if args.obsidian_cli else 1):
                argv = [args.knapper, '-v', str(vault), '-c', str(vault / 'knapper.yaml'), 'dql', case['query'], '--format', 'json', '--timezone', manifest['timezone']]
                if case.get('origin'):
                    argv += ['--origin', case['origin']]
                actual = normalize(json.loads(command(argv)), case)
                entry = {'id': case['id'], 'query': case['query'], 'knapper': actual}
                entry['baselineEqual'] = actual == case['expected']
                if args.obsidian_cli:
                    origin = json.dumps(case.get('origin', ''))
                    observed = evaluate('(async()=>JSON.stringify(await app.plugins.plugins.dataview.api.query(' + json.dumps(case['query']) + ',' + origin + ')))()')
                    if not observed['successful']:
                        raise RuntimeError(observed)
                    entry['obsidian'] = normalize(observed['value'], case)
                    entry['liveEqual'] = actual == entry['obsidian']
                entry['passed'] = (entry.get('liveEqual', True) and (args.watch or entry['baselineEqual']))
                if entry['passed'] or attempt == 9 or not args.obsidian_cli:
                    break
                time.sleep(.5)
            result['comparisons'].append(entry)
        args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
        failed = [r['id'] for r in result['comparisons'] if not r['passed']]
        if failed:
            raise SystemExit('Mismatch: ' + ', '.join(failed) + '; inspect ' + str(args.output))
        iteration += 1
        print(f"pass {iteration}: {len(result['comparisons'])} cases matched", flush=True)
        if not args.watch or (args.iterations and iteration >= args.iterations):
            return
        time.sleep(args.interval)


if __name__ == '__main__':
    try:
        main()
    except KeyboardInterrupt:
        pass
