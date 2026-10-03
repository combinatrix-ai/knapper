import {prepareCapture,surroundCapture} from '../vendor/quickadd/src/formatters/helpers/capturePlacement';
import {protectUserText,restoreUserTextInCapture} from '../vendor/quickadd/src/formatters/helpers/userText';
import {insertAtNoteBodyStartWithResult} from '../vendor/quickadd/src/utils/noteContentInsertion';
(globalThis as any).knapperCapture=async(raw:string)=>{
 try {
  const input=JSON.parse(raw);
  const template=input.template ?? '{{VALUE}}';
  if(template.replace(/\{\{(?:VALUE|CURSOR)\}\}/gi,'').includes('{{') || template.includes('<%'))throw Error('Unsupported QuickAdd template token');
  const expanded=template.replace(/\{\{VALUE\}\}/gi,()=>protectUserText(input.text));
  const payload=restoreUserTextInCapture(prepareCapture(expanded));
  let result=payload;
  if(payload.cursor.kind==='none')result={content:input.before,cursor:payload.cursor};
  else if(input.position==='append') {
   const prefix=input.before.length && !input.before.endsWith('\n')?input.before+'\n':input.before;
   result=surroundCapture(payload,prefix);
  } else if(input.position==='top') {
   const insertion=insertAtNoteBodyStartWithResult(input.before,payload.content);
   result={content:insertion.content,cursor:payload.cursor.kind==='offset' && insertion.insertedStartOffset!==null?{...payload.cursor,value:payload.cursor.value+insertion.insertedStartOffset}: {kind:'none'}};
  } else throw Error('Unsupported capture position');
  return JSON.stringify({text:result.content,cursor:result.cursor,changed:result.content!==input.before});
 }catch(e){return JSON.stringify({error:String(e)});}
};
