const {
  SomeExportedClass,
  SomeAnonymousClass,
  someFunc,
  someAnonymousFunc,
  someArrowFunc,
  someValue,
  someObject,
} = require('./exports.cjs')

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
