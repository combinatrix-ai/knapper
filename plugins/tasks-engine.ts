import './runtime';
import moment from 'moment';
import { Query } from '../vendor/tasks/src/Query/Query';
import { Task } from '../vendor/tasks/src/Task/Task';
import { TaskLocation } from '../vendor/tasks/src/Task/TaskLocation';
import { TasksFile } from '../vendor/tasks/src/Scripting/TasksFile';
import { StatusSettings } from '../vendor/tasks/src/Config/StatusSettings';
import { StatusRegistry } from '../vendor/tasks/src/Statuses/StatusRegistry';
import { resetSettings, updateSettings, getSettings } from '../vendor/tasks/src/Config/Settings';
import { initializeI18n } from '../vendor/tasks/src/i18n/i18n';
(globalThis as any).window = { moment };
(globalThis as any).console = { log() {}, warn() {}, error() {}, debug() {}, info() {}, trace() {} };
(globalThis as any).knapperTasks = async (raw: string) => {
 try {
  const input = JSON.parse(raw);
  // Narrow declared contract: reject host-dependent/custom executable syntax.
  const filter = /^(?:not done|done|is recurring|is not recurring|has (?:due|start|scheduled|created|done|cancelled) date|no (?:due|start|scheduled|created|done|cancelled) date|(?:due|start|scheduled|created|done|cancelled) (?:before|after|on) \d{4}-\d{2}-\d{2}|(?:description|path|folder|filename|heading|tags) (?:includes|does not include) .+|priority is (?:highest|high|medium|normal|low|lowest)|sort by (?:due|start|scheduled|created|done|cancelled|priority|description|path|filename|folder|heading|status)(?: reverse)?|group by (?:due|start|scheduled|created|done|cancelled|priority|path|filename|folder|heading|status)|limit(?: to)? \d+(?: tasks?)?)$/i;
  for (const line of input.query.split(/\r?\n/)) {
   if (!line.trim() || line.trim().startsWith('#')) continue;
   if (line.includes('{{') || line.includes('}}') || !filter.test(line.trim())) throw Error('Unsupported Tasks instruction: ' + line);
  }
  if (input.now) moment.now = () => Date.parse(input.now);
  resetSettings();
  if (input.settings) updateSettings(input.settings);
  StatusSettings.applyToStatusRegistry(getSettings().statusSettings, StatusRegistry.getInstance());
  await initializeI18n();
  const tasks = input.lines.map((l: any) => Task.fromLine({line:l.text,taskLocation:new TaskLocation(new TasksFile(l.path),l.line,0,0,l.heading ?? null),fallbackDate:null})).filter(Boolean);
  const query = new Query(input.query);
  if (query.error) throw Error(query.error);
  const result = query.applyQueryToTasks(tasks);
  if (result.searchErrorMessage) throw Error(result.searchErrorMessage);
  return JSON.stringify({type:'tasks',total:result.totalTasksCount,beforeLimit:result.totalTasksCountBeforeLimit,groups:result.groups.map(g=>({names:g.groups,tasks:g.tasks.map(t=>({path:t.taskLocation.path,line:t.taskLocation.lineNumber+1,status:t.status.symbol,description:t.description,markdown:t.toFileLineString(),due:t.dueDate?.format('YYYY-MM-DD') ?? null,priority:t.priority}))}))});
 } catch(e) {return JSON.stringify({error:String(e)});}
};
