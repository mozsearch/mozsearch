const shorthand = 'shorthand'
const computed = 'computed'
const objectAssign = {}
Object.assign(objectAssign, {
  prop: 0,
  shorthand,
  [computed]: computed,
  method() {
    return [1, 2, 3]
  },
  get accessor() {
    return [1, 2]
  },
  set accessor(val) {
    val
  },
})
objectAssign.prop
objectAssign.shorthand
objectAssign[computed]
objectAssign.method().length
objectAssign.accessor.length
objectAssign.accessor = 'val'
