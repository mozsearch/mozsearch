import java.nio.file.Paths
import kotlin.reflect.full.memberProperties

import org.jetbrains.kotlin.gradle.tasks.KotlinJvmCompile

import org.scip_code.scip_java.kotlin_analysis.KotlinAnalysisIndexer

interface ScipKotlinAnalysisParameters : WorkParameters {
    abstract val sourceroot: DirectoryProperty
    abstract val targetroot: DirectoryProperty
    abstract val libraries: ConfigurableFileCollection
    abstract val sources: ConfigurableFileCollection
}

abstract class ScipKotlinAnalysisAction : WorkAction<ScipKotlinAnalysisParameters> {
    override fun execute() {
        KotlinAnalysisIndexer(
            sourceroot = Paths.get(parameters.sourceroot.get().toString()),
            targetroot = Paths.get(parameters.targetroot.get().toString()),
            sourceRoots = parameters.sources.map {Paths.get(it.path)},
            classpath = parameters.libraries.map {Paths.get(it.path)},
        ).run()
    }
}

abstract class ScipKotlinAnalysisTask: DefaultTask() {
    @get:Inject
    abstract val executor: WorkerExecutor
}

val scipKotlinAnalysis = configurations.create("scipKotlinAnalysis")

dependencies {
    scipKotlinAnalysis("org.scip-code:scip-kotlin-analysis:0.14.1")
}

val analyzeKotlin = tasks.register("analyzeKotlin", ScipKotlinAnalysisTask::class.java) {
    doLast {
        val workQueue = executor.classLoaderIsolation {
            classpath.from(scipKotlinAnalysis)
        }
        for (compileTask in tasks.withType<KotlinJvmCompile>()) {
            workQueue.submit(ScipKotlinAnalysisAction::class) {
                sourceroot = rootProject.layout.projectDirectory
                targetroot = rootProject.layout.buildDirectory.dir("scip-targetroot")
                libraries = compileTask.libraries
                sources = compileTask.sources
            }
        }
    }
}

tasks.withType<KotlinJvmCompile> {
    finalizedBy(analyzeKotlin)
}
