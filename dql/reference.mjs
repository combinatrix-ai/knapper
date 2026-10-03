// Node's native Intl + the same unchanged Dataview parser/importer/evaluator.
import './native-engine.js';
import fs from 'node:fs';
const output = await globalThis.knapperDql(fs.readFileSync(0, 'utf8'));
process.stdout.write(output);
