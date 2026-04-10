const ChromeUtils = {
  defineESModuleGetters(obj, props) {},
  importESModule(module) {},
  defineLazyGetter(obj, prop, getter) {},
}

const obj = {}
ChromeUtils.defineESModuleGetters(obj, {
  constants: 'node:fs',
  SomeExportedClass: './exports.mjs',
})
obj.assert = ChromeUtils.importESModule('node:assert')
ChromeUtils.defineLazyGetter(obj, 'prop', function () {
  return { inner: 0 }
})

obj.constants.O_CREAT
obj.assert.equal
obj.prop.inner
new obj.SomeExportedClass().method()
