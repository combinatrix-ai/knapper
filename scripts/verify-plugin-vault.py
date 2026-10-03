#!/usr/bin/env python3
"""Compare pinned community-plugin cases in an isolated disposable Obsidian vault.

Linter/Capture probes write only the synthetic PluginLinter/PluginCapture notes.
Requires explicit --allow-fixture-writes and refuses the committed fixture root.
"""
import argparse
import json
import os
import pathlib
import re
import subprocess

TASKS_CODE = '''(async()=>{const p=app.plugins.plugins['obsidian-tasks-plugin'];let child;const el=document.createElement('div');await p.queryRenderer.addQueryRenderChild(QUERY,el,{sourcePath:'PluginTasks/Plugin.md',addChild:c=>{child=c;}});try{const q=child.queryResultsRenderer.query;if(q.error)throw Error(q.error);const r=q.applyQueryToTasks(p.getTasks());return JSON.stringify({type:'tasks',total:r.totalTasksCount,beforeLimit:r.totalTasksCountBeforeLimit,groups:r.groups.map(g=>({names:g.groups,tasks:g.tasks.map(t=>({path:t.taskLocation.path,line:t.taskLocation.lineNumber+1,status:t.status.symbol,description:t.description,markdown:t.toFileLineString(),due:t.dueDate?.format('YYYY-MM-DD')??null,priority:t.priority}))}))});}finally{child.unload();}})()'''
LINTER_CODE = '''(()=>{const p=app.plugins.plugins['obsidian-linter'];const settings=JSON.parse(JSON.stringify(p.settings));for(const r of Object.values(settings.ruleConfigs))r.enabled=false;for(const name of RULES)settings.ruleConfigs[name].enabled=true;settings.customRegexes=[];return JSON.stringify(p.rulesRunner.lintText({oldText:TEXT,fileInfo:{name:'Input',createdAtFormatted:'2026-10-03',modifiedAtFormatted:'2026-10-03',path:'PluginLinter/Input.md'},settings,momentLocale:'en',getCurrentTime:()=>window.moment('2026-10-03'),defaultMisspellings:new Map()}));})()'''
QUICKADD_CODE = '''(async()=>{const path='PluginCapture/Log.md';if(!app.vault.getAbstractFileByPath('PluginCapture'))await app.vault.createFolder('PluginCapture');let file=app.vault.getFileByPath(path);if(file)await app.vault.modify(file,BEFORE);else file=await app.vault.create(path,BEFORE);const p=app.plugins.plugins.quickadd;const original=p.settings.choices;const choice={id:'knapper-capture-test',name:'knapper-capture-test',type:'Capture',command:false,onePageInput:'never',captureTo:path,captureToActiveFile:false,activeFileWritePosition:'cursor',createFileIfItDoesntExist:{enabled:false,createWithTemplate:false,template:''},format:{enabled:true,format:TEMPLATE},insertAfter:{enabled:false,after:'',insertAtEnd:false},insertBefore:{enabled:false,before:''},newLineCapture:{enabled:false,direction:'below'},prepend:POSITION==='append',task:false,openFile:false,appendLink:false,copyLinkToClipboard:false,templater:{afterCapture:'none'}};p.settings.choices=[choice];try{await p.api.executeChoice(choice.name,{value:TEXT});return JSON.stringify(await app.vault.read(file));}finally{p.settings.choices=original;}})()'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--knapper', required=True)
    parser.add_argument('--obsidian-cli', required=True)
    parser.add_argument('--vault', required=True, type=pathlib.Path)
    parser.add_argument('--output', required=True, type=pathlib.Path)
    parser.add_argument('--allow-fixture-writes', action='store_true')
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parent.parent
    vault = args.vault.resolve()
    if not args.allow_fixture_writes or vault == (root / 'tests/fixtures/pkm').resolve():
        parser.error('use a disposable fixture copy and --allow-fixture-writes')
    env = {**os.environ, 'KNAPPER_NO_UPDATE_CHECK': '1'}

    def command(argv):
        return subprocess.check_output(argv, text=True, timeout=60, env=env).strip()

    def evaluate(code):
        raw = command([args.obsidian_cli, 'vault=' + vault.name, 'eval', 'code=' + code])
        if not raw.startswith('=> '):
            raise RuntimeError(raw)
        return json.loads(raw[3:])

    info = evaluate('JSON.stringify({path:app.vault.adapter.getBasePath(),versions:Object.fromEntries(["obsidian-tasks-plugin","obsidian-linter","quickadd"].map(id=>[id,app.plugins.plugins[id]?.manifest.version]))})')
    if pathlib.Path(info['path']).resolve() != vault:
        raise RuntimeError('CLI connected to a different vault')
    records = []
    for kind, filename, plugin in [('tasks', 'tasks-plugin-cases.json', 'obsidian-tasks-plugin'), ('linter', 'linter-cases.json', 'obsidian-linter'), ('quickadd', 'quickadd-cases.json', 'quickadd')]:
        manifest = json.loads((vault / filename).read_text())
        if info['versions'].get(plugin) != manifest['pluginVersion']:
            raise RuntimeError('Plugin version mismatch: ' + plugin)
        for case in manifest['cases']:
            if kind == 'tasks':
                code = TASKS_CODE.replace('QUERY', json.dumps(case['query']))
                argv = ['tasks', '--query', case['query'], '--format', 'json']
                expected = evaluate(code)
            elif kind == 'linter':
                code = LINTER_CODE.replace('RULES', json.dumps(case['rules'])).replace('TEXT', json.dumps(case['before']))
                expected = evaluate(code)
                target = vault / 'PluginLinter/Input.md'
                target.parent.mkdir(exist_ok=True)
                target.write_text(case['before'])
                argv = ['format-note', 'PluginLinter/Input.md', '--format', 'json'] + [part for rule in case['rules'] for part in ['--rule', rule]]
            else:
                replacements = {'BEFORE': case['before'], 'TEMPLATE': case['template'], 'POSITION': case['position'], 'TEXT': case['text']}
                code = re.sub(r'\b(BEFORE|TEMPLATE|POSITION|TEXT)\b', lambda m: json.dumps(replacements[m[0]]), QUICKADD_CODE)
                expected = evaluate(code)
                (vault / 'PluginCapture/Log.md').write_text(case['before'])
                (vault / 'PluginCapture/Template.md').write_text(case['template'])
                argv = ['capture', 'PluginCapture/Log.md', '--text', case['text'], '--template', 'PluginCapture/Template.md', '--position', case['position'], '--format', 'json']
            actual = json.loads(command([args.knapper, '-v', str(vault), '-c', str(vault / 'knapper.yaml'), *argv]))
            if kind != 'tasks':
                actual = actual['after']
            records.append({'id': case['id'], 'plugin': plugin, 'obsidian': expected, 'knapper': actual, 'liveEqual': actual == expected, 'baselineEqual': expected == case['expected']})
    result = {'appVersion': command([args.obsidian_cli, 'version']), 'versions': info['versions'], 'comparisons': records}
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    failed = [c['id'] for c in records if not c['liveEqual'] or not c['baselineEqual']]
    if failed:
        raise SystemExit('Mismatch: ' + ', '.join(failed))
    print(f'{len(records)} plugin cases matched actual App and recorded expectations')


if __name__ == '__main__':
    main()
