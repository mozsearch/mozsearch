export class SomeExportedClass {
  method() {}
}

export const SomeAnonymousClass = class /*anonymous*/ {
  method() {}
}

export function someFunc() {}
export const someAnonymousFunc = function () /*anonymous*/ {}

export const someArrowFunc = () => {}

export const someValue = 4

export const someObject = {
  value: 5,
  method() {},
  get prop() {
    return true
  },
  set prop(val) {},
}
