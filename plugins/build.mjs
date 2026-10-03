import {build} from 'esbuild';
import path from 'node:path';
import fs from 'node:fs';
const here=path.dirname(new URL(import.meta.url).pathname);
const root=path.resolve(here,'..');
const licenses=new Map();
for(const name of ['tasks-engine','linter-engine','quickadd-engine','upstream-tests']) {
 const result=await build({absWorkingDir:root,entryPoints:[path.join(here,name+'.ts')],outfile:path.join(here,name+'.js'),bundle:true,format:'iife',platform:'neutral',mainFields:['module','main'],target:'es2020',minify:true,nodePaths:[path.join(here,'node_modules')],alias:{obsidian:path.join(here,'obsidian.ts'),vitest:path.join(here,'vitest.ts')},metafile:true,plugins:[{name:'headless-options',setup(b){b.onResolve({filter:/^\.\/common$/},args=>args.importer.includes('/vendor/linter/__tests__/')?{path:path.join(here,'linter-test-common.ts')}:undefined);b.onResolve({filter:/^\.\.?\/option$/},args=>args.importer.includes('/vendor/linter/')?{path:path.join(here,'linter-options.ts')}:undefined);}}]});
 fs.writeFileSync(path.join(here,name+'.meta.json'),JSON.stringify(result.metafile,null,2)+'\n');
 for(const input of Object.keys(result.metafile.inputs)) {
  if(!input.includes('node_modules'))continue;
  let dir=path.dirname(path.resolve(root,input));
  while(dir.includes('node_modules')) {
   const file=path.join(dir,'package.json');
   if(fs.existsSync(file)) {
    const pkg=JSON.parse(fs.readFileSync(file));
    if(pkg.name) {
     if(!['MIT','BSD-3-Clause','0BSD','ISC','Apache-2.0','BlueOak-1.0.0'].includes((pkg.license ?? pkg.licenses?.[0]?.type)))throw Error('Unaudited license: '+pkg.name+' '+pkg.license);
     const notices=fs.readdirSync(dir).filter(f=>/^(license|licence|copying|notice)([.-]|$)/i.test(f));
     if(!notices.length && pkg.name==='format' && pkg.version==='0.2.2')notices.push('Readme.md');
     if(!notices.length)throw Error('Missing notice: '+pkg.name);
     licenses.set(pkg.name+'@'+pkg.version,notices.map(f=>fs.readFileSync(path.join(dir,f),'utf8')).join('\n') + (pkg.name==='format' ? '\nMIT license text (as declared in package.json/Readme.md):\n'+fs.readFileSync(path.join(root,'vendor/tasks/LICENSE'),'utf8').split('Permission is hereby granted')[1].replace(/^/, 'Permission is hereby granted') : ''));
     break;
    }
   }
   dir=path.dirname(dir);
  }
 }
}
fs.writeFileSync(path.join(here,'THIRD_PARTY_NOTICES.txt'),'Obsidian Tasks @ 692e965ecbaad197221fae9ddff13f5c5fa6ece6\n'+fs.readFileSync(path.join(root,'vendor/tasks/LICENSE'),'utf8')+'\n\nObsidian Linter @ b15df18a182bbbc750209a8913a89469a164d01a\n'+fs.readFileSync(path.join(root,'vendor/linter/LICENSE'),'utf8')+'\n\nQuickAdd @ 943649ddb105ee166c52b44222e7178d7632a98a\n'+fs.readFileSync(path.join(root,'vendor/quickadd/LICENSE'),'utf8')+'\n\n'+[...licenses].sort(([a],[b])=>a.localeCompare(b)).map(([p,l])=>p+'\n'+l).join('\n\n'));
