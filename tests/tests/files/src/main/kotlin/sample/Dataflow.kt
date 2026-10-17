/*
 * Fixtures for following values (see tools/src/dataflow/ and the checks in
 * tests/tests/checks/inputs/dataflow/): locals, parameters, properties, and
 * an interface's implementation.
 */
package sample

class DataflowSettings {
    var url: String = ""
}

interface UrlSink {
    fun setUrl(url: String)
}

class DataflowProvider(private val settings: DataflowSettings) : UrlSink {
    override fun setUrl(url: String) {
        settings.url = url
    }

    fun currentUrl(): String {
        return settings.url
    }
}

fun normalizeUrl(input: String): String {
    val trimmed = input.trim()
    return if (trimmed.startsWith("https://")) trimmed else "https://" + trimmed
}

fun configure(sink: UrlSink, input: String) {
    val normalized = normalizeUrl(input)
    sink.setUrl(normalized)
}
