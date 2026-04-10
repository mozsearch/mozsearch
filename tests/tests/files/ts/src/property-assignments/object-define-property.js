const objectDefineProperty = {}
Object.defineProperty(objectDefineProperty, 'prop', 0)
Object.defineProperty(objectDefineProperty, 'accessors', {
  get() {
    return true
  },
  set(val) {
    val
  },
})
objectDefineProperty.prop
objectDefineProperty.accessors
objectDefineProperty.accessors = 1
