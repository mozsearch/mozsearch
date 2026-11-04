plugins {
    `kotlin-dsl`
}

repositories {
    mavenCentral()
}

dependencies {
    compileOnly("org.scip-code:scip-kotlin-bindings:0.10.0")
    compileOnly("org.scip-code:scip-kotlin-analysis:0.14.1")
    implementation("org.jetbrains.kotlin:kotlin-gradle-plugin-api:2.4.20")
}
