import java.nio.file.Paths
import org.jetbrains.kotlin.gradle.tasks.KotlinJvmCompile
import org.scip_code.scip_java.kotlin_analysis.KotlinAnalysisIndexer

interface ScipKotlinAnalysisParameters : WorkParameters {
    val sourceroot: Property<String>
    val targetroot: Property<String>
    val libraries: ConfigurableFileCollection
    val sources: ConfigurableFileCollection
}

abstract class ScipKotlinAnalysisAction : WorkAction<ScipKotlinAnalysisParameters> {
    override fun execute() {
        KotlinAnalysisIndexer(
            sourceroot = Paths.get(parameters.sourceroot.get()),
            targetroot = Paths.get(parameters.targetroot.get()),
            sourceRoots = parameters.sources.map {Paths.get(it.path)},
            classpath = parameters.libraries.map {Paths.get(it.path)},
        ).run()
    }
}

abstract class ScipKotlinAnalysisTask: DefaultTask() {
    @get:Inject
    abstract val executor: WorkerExecutor

    @get:Classpath
    abstract val analysisClasspath: ConfigurableFileCollection

    @get:Input
    abstract val sourceroot: Property<String>

    @get:Input
    abstract val targetroot: Property<String>

    @get:InputFiles
    abstract val libraries: ConfigurableFileCollection

    @get:InputFiles
    abstract val sources: ConfigurableFileCollection

    @TaskAction
    fun run() {
        val task = this

        executor.processIsolation {
            classpath.from(analysisClasspath)
        }.submit(ScipKotlinAnalysisAction::class.java) {
            sourceroot.set(task.sourceroot)
            targetroot.set(task.targetroot)
            libraries.from(task.libraries)
            sources.from(task.sources)
        }
    }
}

var topsrcdir = rootProject.projectDir.getCanonicalPath()
var targetdir = rootProject.layout.buildDirectory.file("scip-targetroot").get()

val scipJavacConfig = project.configurations.register("scipJavac")
val scipKotlinAnalysisConfig = project.configurations.register("scipKotlinAnalysis")
dependencies {
    scipJavacConfig("org.scip-code:scip-javac:0.14.1")
    scipKotlinAnalysisConfig("org.scip-code:scip-kotlin-analysis:0.14.1")
}

project.tasks.withType(JavaCompile::class.java).configureEach {
    val pluginClasspath = project.files(scipJavacConfig)
    doFirst {
        classpath += pluginClasspath
    }
    options.compilerArgs.add("-Xplugin:scip -sourceroot:${topsrcdir} -targetroot:${targetdir}")
}

project.tasks.withType(KotlinJvmCompile::class.java) {
    val compileTask = this

    val scipTask = tasks.register("${name}Scip", ScipKotlinAnalysisTask::class.java) {
        analysisClasspath.from(scipKotlinAnalysisConfig)

        sourceroot.set(topsrcdir)
        targetroot.set(targetdir.toString())
        libraries.from(compileTask.libraries)
        sources.from(compileTask.sources)
    }

    finalizedBy(scipTask)
}
