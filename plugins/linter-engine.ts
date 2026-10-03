import './runtime';
import TrailingSpaces from '../vendor/linter/src/rules/trailing-spaces';
import RemoveMultipleSpaces from '../vendor/linter/src/rules/remove-multiple-spaces';
import HeadingBlankLines from '../vendor/linter/src/rules/heading-blank-lines';
import {getDisabledRules} from '../vendor/linter/src/rules';
const builders:any={'trailing-spaces':TrailingSpaces,'remove-multiple-spaces':RemoveMultipleSpaces,'heading-blank-lines':HeadingBlankLines};
(globalThis as any).knapperLinter=async(raw:string)=>{
 try {
  const input=JSON.parse(raw);let text=input.text;
  const [disabled,all]=getDisabledRules(text);
  if(!all)for(const name of input.rules) {
   if(!builders[name])throw Error('Unsupported Linter rule: '+name);
   if(!disabled.includes(name))text=builders[name].getRule().apply(text);
  }
  return JSON.stringify({text,changed:text!==input.text});
 }catch(e){return JSON.stringify({error:String(e)});}
};
