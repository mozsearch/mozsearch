package sample

interface Interface {
    fun someFunction()
    fun someOtherFunction(): Boolean
}

class JavaLibraryTest {
    val a = object : Interface {
        override fun someFunction() {}
        override fun someOtherFunction(): Boolean {
            return true;
        }
    }
    val b = object : Interface {
        override fun someFunction() {}
        override fun someOtherFunction() = false
    }
}
