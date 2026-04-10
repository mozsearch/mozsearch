import {
  SomeExportedClass,
  SomeAnonymousClass,
  someFunc,
  someAnonymousFunc,
  someArrowFunc,
  someValue,
  someObject,
} from './exports.mjs'

new SomeExportedClass().method()
new SomeAnonymousClass().method()
someFunc()
someAnonymousFunc()
someArrowFunc()
someValue
someObject.value
someObject.method()
someObject.prop
someObject.prop = 3
