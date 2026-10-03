// The upstream suites use only these synchronous Jest matchers. Unsupported
// matchers fail immediately; test bodies and expectations are not modified.
const failures: {name:string,error:string}[] = [];
let total = 0, passed = 0;
const scopes: string[] = [];
function equal(a:any,b:any, seen = new Map<any, any>()): boolean {
  if (Object.is(a,b)) return true;
  if (a == null || b == null || typeof a !== 'object' || typeof b !== 'object') return false;
  if (seen.has(a)) return seen.get(a) === b;
  seen.set(a,b);
  if (a instanceof Date || b instanceof Date) return a instanceof Date && b instanceof Date && a.getTime() === b.getTime();
  if (a instanceof Set || b instanceof Set) return a instanceof Set && b instanceof Set && a.size === b.size && [...a].every(x => [...b].some(y => equal(x,y,new Map(seen))));
  if (a instanceof Map || b instanceof Map) return a instanceof Map && b instanceof Map && a.size === b.size && [...a].every(([k,v]) => [...b].some(([bk,bv]) => equal(k,bk,new Map(seen)) && equal(v,bv,new Map(seen))));
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ak = Object.keys(a).filter(k => a[k] !== undefined), bk = Object.keys(b).filter(k => b[k] !== undefined);
  return ak.length === bk.length && ak.every(k => Object.hasOwn(b,k) && equal(a[k],b[k],new Map(seen)));
}
function display(value:any): string {try { return JSON.stringify(value); } catch {return String(value);}}
(globalThis as any).describe = (name:string, body:()=>void) => {scopes.push(name); try {body();} finally {scopes.pop();}};
const tests: {name:string,body:()=>any}[] = [];
(globalThis as any).test = (name:string, body:()=>any) => tests.push({name:[...scopes,name].join(' / '),body});
(globalThis as any).expect = (actual:any) => {
  const check = (condition:boolean, matcher:string, expected?:any) => {if (!condition) throw Error(matcher + ': actual=' + display(actual) + ', expected=' + display(expected));};
  return {
    toEqual: (expected:any) => check(equal(actual,expected),'toEqual',expected),
    toBe: (expected:any) => check(Object.is(actual,expected),'toBe',expected),
    toBeNull: () => check(actual === null,'toBeNull'),
    toBeTruthy: () => check(!!actual,'toBeTruthy'),
    toBeFalsy: () => check(!actual,'toBeFalsy'),
    toBeLessThan: (expected:number) => check(actual < expected,'toBeLessThan',expected),
    toBeCloseTo: (expected:number, digits = 2) => check(Math.abs(actual-expected) < 0.5 * 10**(-digits),'toBeCloseTo',expected),
  };
};
(globalThis as any).knapperTestReport = () => {
  for (const {name,body} of tests.splice(0)) {
    total++;
    try {const result = body(); if (result && typeof result.then === 'function') throw Error('Async upstream test needs a supported runner'); passed++;}
    catch (err:any) {failures.push({name,error:String(err.stack ?? err)});}
  }
  return JSON.stringify({total,passed,failures});
};
