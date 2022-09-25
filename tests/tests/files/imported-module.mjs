export default function moduleDefaultFunc() {
    return "I'm the default!";
}

export const moduleConst = "I am a constant";

export function moduleFunc(moduleFuncArg1) {
    console.log("My argument was:", moduleFuncArg1);
}

export class ModuleClass {
    // Our preprocessor logic should not freak out about this.
    #error = null;

    constructor() {
        this.moduleClassField = 1;
    }
}
