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
}

group = "io.agntcy.slim"
version = "0.7.0"

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
