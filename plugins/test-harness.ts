// Synchronous runner for the explicitly selected unchanged upstream tests.
const cases: {name:string, body:()=>void}[]=[];
(globalThis as any).describe = (_name:string,body:()=>void)=>body();
const it = (name:string,body:()=>void)=>cases.push({name,body});
(it as any).each=(values:any[])=>(name:string,body:(...args:any[])=>void)=>values.forEach((v,i)=>it(name+' #'+i,()=>body(...(Array.isArray(v)?v:[v]))));
(globalThis as any).it=it;
(globalThis as any).expect=(actual:any)=>{
 const equal=(a:any,b:any):boolean=>{
  if(Object.is(a,b))return true;
  if(a===null || b===null || typeof a!=='object' || typeof b!=='object')return false;
  if(Array.isArray(a)!==Array.isArray(b))return false;
  const keys=Object.keys(a),other=Object.keys(b);
  return keys.length===other.length && keys.every(k=>Object.hasOwn(b,k) && equal(a[k],b[k]));
 };
 const compare=(expected:any)=>equal(actual,expected);
 const check=(value:boolean)=>{if(!value)throw Error('unexpected '+JSON.stringify(actual));};
 return {toBe:(expected:any)=>check(Object.is(actual,expected)),toEqual:(expected:any)=>check(compare(expected)),toBeNull:()=>check(actual===null),not:{toMatch:(pattern:RegExp)=>check(!pattern.test(actual)),toEqual:(expected:any)=>check(!compare(expected)),toBeNull:()=>check(actual!==null)}};
};
(globalThis as any).knapperTestReport=()=>{
 const failures:any[]=[];
 for(const {name,body} of cases)try{body();}catch(e){failures.push({name,error:String(e)});}
 return JSON.stringify({total:cases.length,passed:cases.length-failures.length,failures});
};
