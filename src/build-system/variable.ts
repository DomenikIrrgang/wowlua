export class Variable {

    public constructor(
        public name: string,
        public global: boolean,
        public type: VariableType,
        public value?: string | number | boolean | Variable | Variable[],
        public parent?: Variable,
    ) {}

    public getFullName(): string {
        if (this.parent) {
            return this.parent.getFullName() + ":" + this.name
        } else {
            return this.name
        }
    }
    
}

export enum VariableType {
    STRING = "string",
    NUMBER = "number",
    BOOLEAN = "boolean",
    TABLE = "table",
    FUNCTION = "function",
    NIL = "nil",
    ANY = "any",
}