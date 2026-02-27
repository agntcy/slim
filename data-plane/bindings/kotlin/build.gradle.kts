// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import org.jetbrains.kotlin.gradle.tasks.KotlinCompile
import org.gradle.api.attributes.Usage
import org.gradle.api.attributes.java.TargetJvmEnvironment

plugins {
    kotlin("jvm") version "2.1.0"
    kotlin("plugin.serialization") version "2.1.0"
    application
    `maven-publish`
    signing
}

group = "io.agntcy.slim"
version = "1.0.0"

repositories {
    mavenCentral()
}

dependencies {
    // UniFFI-generated Kotlin bindings depend on JNA for native library loading
    implementation("net.java.dev.jna:jna:5.14.0")
    
    // Kotlin coroutines for async support
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.1")
    
    // CLI argument parsing
    implementation("com.github.ajalt.clikt:clikt:4.2.2")
    
    // JSON parsing for config files
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.6.3")
    
    // Testing
    testImplementation(kotlin("test"))
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.8.1")
    testImplementation("org.junit.jupiter:junit-jupiter-params:5.10.1")
}

// Configure source sets to include generated code
sourceSets {
    main {
        kotlin {
            srcDirs("src/main/kotlin", "generated", "examples", "examples/common")
        }
        resources {
            srcDirs("generated/jniLibs")
        }
    }
    test {
        resources {
            srcDirs("src/test/resources")
        }
    }
}

// Configure test resource processing
tasks.processTestResources {
    duplicatesStrategy = DuplicatesStrategy.INCLUDE
}

// Compile for Java 17 bytecode (works with Java 17, 21, 23, 25+)
tasks.withType<KotlinCompile> {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

tasks.withType<JavaCompile> {
    sourceCompatibility = "17"
    targetCompatibility = "17"
}

tasks.test {
    useJUnitPlatform()
    
    // Show test output
    testLogging {
        events("passed", "skipped", "failed", "standardOut", "standardError")
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
        showExceptions = true
        showCauses = true
        showStackTraces = true
    }
}

// Task to run all tests with verbose output
tasks.register<Test>("testVerbose") {
    group = "verification"
    description = "Run all tests with verbose output"
    useJUnitPlatform()
    
    testLogging {
        events("passed", "skipped", "failed", "standardOut", "standardError")
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
        showExceptions = true
        showCauses = true
        showStackTraces = true
        showStandardStreams = true
    }
}

// Task to run a specific test class
tasks.register<Test>("testClass") {
    group = "verification"
    description = "Run a specific test class (use -PtestClass=ClassName)"
    useJUnitPlatform()
    
    if (project.hasProperty("testClass")) {
        val testClassName = project.property("testClass").toString()
        filter {
            includeTestsMatching("io.agntcy.slim.bindings.$testClassName")
        }
    }
    
    testLogging {
        events("passed", "skipped", "failed", "standardOut", "standardError")
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
        showExceptions = true
        showCauses = true
        showStackTraces = true
        showStandardStreams = true
    }
}

// Task to run tests matching a pattern
tasks.register<Test>("testPattern") {
    group = "verification"
    description = "Run tests matching a pattern (use -PtestPattern=*pattern*)"
    useJUnitPlatform()
    
    if (project.hasProperty("testPattern")) {
        val pattern = project.property("testPattern").toString()
        filter {
            includeTestsMatching(pattern)
        }
    }
    
    testLogging {
        events("passed", "skipped", "failed", "standardOut", "standardError")
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
        showExceptions = true
        showCauses = true
        showStackTraces = true
        showStandardStreams = true
    }
}

// Task to run the Server example
tasks.register<JavaExec>("runServer") {
    group = "application"
    description = "Run the SLIM server example"
    classpath = sourceSets["main"].runtimeClasspath
    mainClass.set("io.agntcy.slim.examples.ServerKt")
    standardInput = System.`in`
    
    // Pass CLI args
    if (project.hasProperty("args")) {
        args(project.property("args").toString().split(" "))
    }
}

// Task to run the Point-to-Point example
tasks.register<JavaExec>("runPointToPoint") {
    group = "application"
    description = "Run the point-to-point messaging example"
    classpath = sourceSets["main"].runtimeClasspath
    mainClass.set("io.agntcy.slim.examples.PointToPointKt")
    standardInput = System.`in`
    
    // Pass CLI args
    if (project.hasProperty("args")) {
        args(project.property("args").toString().split(" "))
    }
}

// Task to run the Group example
tasks.register<JavaExec>("runGroup") {
    group = "application"
    description = "Run the group messaging example"
    classpath = sourceSets["main"].runtimeClasspath
    mainClass.set("io.agntcy.slim.examples.GroupKt")
    standardInput = System.`in`
    
    // Pass CLI args
    if (project.hasProperty("args")) {
        args(project.property("args").toString().split(" "))
    }
}

// Build fat JARs for each example
tasks.register<Jar>("serverJar") {
    group = "build"
    description = "Create a fat JAR for the server example"
    archiveBaseName.set("slim-server")
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    
    manifest {
        attributes["Main-Class"] = "io.agntcy.slim.examples.ServerKt"
    }
    
    from(sourceSets.main.get().output)
    dependsOn(configurations.runtimeClasspath)
    from({
        configurations.runtimeClasspath.get().filter { it.name.endsWith("jar") }.map { zipTree(it) }
    })
}

tasks.register<Jar>("pointToPointJar") {
    group = "build"
    description = "Create a fat JAR for the point-to-point example"
    archiveBaseName.set("slim-point-to-point")
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    
    manifest {
        attributes["Main-Class"] = "io.agntcy.slim.examples.PointToPointKt"
    }
    
    from(sourceSets.main.get().output)
    dependsOn(configurations.runtimeClasspath)
    from({
        configurations.runtimeClasspath.get().filter { it.name.endsWith("jar") }.map { zipTree(it) }
    })
}

tasks.register<Jar>("groupJar") {
    group = "build"
    description = "Create a fat JAR for the group example"
    archiveBaseName.set("slim-group")
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    
    manifest {
        attributes["Main-Class"] = "io.agntcy.slim.examples.GroupKt"
    }
    
    from(sourceSets.main.get().output)
    dependsOn(configurations.runtimeClasspath)
    from({
        configurations.runtimeClasspath.get().filter { it.name.endsWith("jar") }.map { zipTree(it) }
    })
}

// ==========================================================================
// Maven Central Publishing Configuration
// ==========================================================================

// Extract version from project property or tag (set via -PpublishVersion=x.y.z in CI)
val publishVersion = project.findProperty("publishVersion") as String? ?: version.toString()

// Sources JAR (required by Maven Central)
val sourcesJar by tasks.registering(Jar::class) {
    archiveClassifier.set("sources")
    from(sourceSets.main.get().allSource)
}

// Javadoc JAR (required by Maven Central, can be minimal for Kotlin)
val javadocJar by tasks.registering(Jar::class) {
    archiveClassifier.set("javadoc")
    from("$projectDir/README.md")
}

publishing {
    publications {
        create<MavenPublication>("maven") {
            groupId = "io.agntcy.slim"
            artifactId = "slim-bindings-kotlin"
            version = publishVersion
            
            from(components["java"])
            artifact(sourcesJar)
            artifact(javadocJar)
            
            pom {
                name.set("SLIM Kotlin Bindings")
                description.set("Kotlin bindings for SLIM (Secure Low-Latency Interactive Messaging)")
                url.set("https://github.com/agntcy/slim")
                
                licenses {
                    license {
                        name.set("Apache-2.0")
                        url.set("https://www.apache.org/licenses/LICENSE-2.0")
                    }
                }
                
                developers {
                    developer {
                        name.set("AGNTCY Contributors")
                        url.set("https://github.com/agntcy")
                    }
                }
                
                scm {
                    url.set("https://github.com/agntcy/slim")
                    connection.set("scm:git:git://github.com/agntcy/slim.git")
                    developerConnection.set("scm:git:ssh://git@github.com/agntcy/slim.git")
                }
            }
        }
    }
    
    repositories {
        maven {
            name = "GitHubPackages"
            url = uri("https://maven.pkg.github.com/agntcy/slim")
            credentials {
                username = System.getenv("GITHUB_ACTOR")
                password = System.getenv("GITHUB_TOKEN")
            }
        }
        // Maven Central via OSSRH Staging API (central.sonatype.com). OSSRH (s01.oss.sonatype.org) was shut down June 2025.
        // Requires Central Portal User Token: https://central.sonatype.org/publish/generate-portal-token/
        if (System.getenv("MVN_TOKEN_NAME") != null && System.getenv("MVN_TOKEN_PASSWORD") != null) {
            maven {
                name = "MavenCentral"
                url = uri("https://ossrh-staging-api.central.sonatype.com/service/local/staging/deploy/maven2/")
                credentials {
                    username = System.getenv("MVN_TOKEN_NAME")
                    password = System.getenv("MVN_TOKEN_PASSWORD")
                }
            }
        }
    }
}

// GPG signing for Maven Central (required). Uses in-memory key when env vars are set.
if (System.getenv("GPG_SIGNING_KEY") != null && System.getenv("GPG_PASSPHRASE") != null) {
    signing {
        useInMemoryPgpKeys(System.getenv("GPG_SIGNING_KEY"), System.getenv("GPG_PASSPHRASE"))
        sign(publishing.publications["maven"])
    }
}
