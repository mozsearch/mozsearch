const objectDefineProperties = {}
Object.defineProperties(objectDefineProperties, {
  prop: 0,
  accessors: {
    get() {
      return true
    },
    set(val) {
      val
    },
  },
})
objectDefineProperties.prop
objectDefineProperties.accessors
objectDefineProperties.accessors = 1
