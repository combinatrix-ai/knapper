// Only option metadata is used headlessly. Obsidian setting controls are omitted.
export class Option {
 constructor(public configKey:string,public nameKey:string,public descriptionKey:string,public defaultValue:any,public ruleAlias?:string,..._ui:any[]) {}
}
export class BooleanOption extends Option {}
export class DropdownOption extends Option {}
export class MdFilePickerOption extends Option {}
export class MomentFormatOption extends Option {}
export class ListItemOption extends Option {}
export class TextOption extends Option {}
export class DropdownRecord {constructor(public value:string,public descriptionKey:string) {}}
