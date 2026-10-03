export function ruleTest(args:any) {
 const rule=args.RuleBuilderClass.getRule();
 (globalThis as any).describe(rule.getName(),()=>{
  for(const c of args.testCases)(globalThis as any).it(c.testName,()=>{
   const options=typeof c.options==='function'?c.options():c.options;
   (globalThis as any).expect(rule.apply(c.before,options)).toEqual(c.after);
   c.afterTestFunc?.call(c);
  });
 });
}
