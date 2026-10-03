import fs from 'node:fs';
import vm from 'node:vm';
vm.runInThisContext(fs.readFileSync(new URL('./upstream-tests.js',import.meta.url),'utf8'));
const report=JSON.parse(globalThis.knapperTestReport());
process.stdout.write(JSON.stringify(report)+'\n');
if(report.total!==134 || report.failures.length)process.exitCode=1;
