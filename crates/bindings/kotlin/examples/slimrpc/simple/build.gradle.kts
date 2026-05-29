// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import org.jetbrains.kotlin.gradle.tasks.KotlinCompile

plugins {
    kotlin("jvm") version "2.2.0"
    application
}

group = "com.example_service"
version = "0.1.0"

repositories {
    mavenCentral()
    mavenLocal()
    maven {
        name = "GitHubPackages"
        url = uri("https://maven.pkg.github.com/agntcy/slim")
        credentials {
            username = System.getenv("GITHUB_ACTOR") ?: ""
            password = System.getenv("GITHUB_TOKEN") ?: ""
        }
    }
}

val slimBindingsVersion = "1.0.0"
val protobufVersion = "4.34.1"
val jnaVersion = "5.14.0"
val coroutinesVersion = "1.8.1"

dependencies {
    implementation("io.agntcy.slim:slim-bindings-kotlin:$slimBindingsVersion")
    implementation("com.google.protobuf:protobuf-java:$protobufVersion")
    implementation("com.google.protobuf:protobuf-kotlin:$protobufVersion")
    implementation("net.java.dev.jna:jna:$jnaVersion")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:$coroutinesVersion")
}

sourceSets {
    main {
        java {
            srcDirs("types")
        }
        kotlin {
            srcDirs("src/main/kotlin", "types", "slimrpc")
        }
    }
}

tasks.withType<KotlinCompile> {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

tasks.withType<JavaCompile> {
    sourceCompatibility = "17"
    targetCompatibility = "17"
}

tasks.register<JavaExec>("server") {
    group = "application"
    description = "Run the slimrpc server example"
    classpath = sourceSets["main"].runtimeClasspath
    mainClass.set("com.example_service.example.server.SlimrpcServerMainKt")

    if (project.hasProperty("args")) {
        args(project.property("args").toString().split(" "))
    }
}

tasks.register<JavaExec>("client") {
    group = "application"
    description = "Run the slimrpc client example"
    classpath = sourceSets["main"].runtimeClasspath
    mainClass.set("com.example_service.example.client.SlimrpcClientMainKt")

    if (project.hasProperty("args")) {
        args(project.property("args").toString().split(" "))
    }
}

tasks.register<JavaExec>("groupClient") {
    group = "application"
    description = "Run the slimrpc group client example"
    classpath = sourceSets["main"].runtimeClasspath
    mainClass.set("com.example_service.example.client.SlimrpcGroupClientMainKt")

    if (project.hasProperty("args")) {
        args(project.property("args").toString().split(" "))
    }
}
