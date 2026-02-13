// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for SLIM Node bindings.
 * This file is a stub; full types are generated from generated/slim-bindings-node.ts
 * by running task node:emit-types (before publish).
 */
export function getVersion(): string;
export function isInitialized(): boolean;
export function initializeWithDefaults(): void;
export function shutdownBlocking(): void;
export type RuntimeConfig = Record<string, unknown>;
export type ServiceConfig = Record<string, unknown>;
export type DataplaneConfig = Record<string, unknown>;
export type TracingConfig = Record<string, unknown>;
export function newRuntimeConfig(): RuntimeConfig;
export function newTracingConfig(): TracingConfig;
export function newServiceConfig(): ServiceConfig;
export function newDataplaneConfig(): DataplaneConfig;
export function initializeWithConfigs(
  runtimeConfig: RuntimeConfig,
  tracingConfig: TracingConfig,
  serviceConfig: ServiceConfig[]
): void;
export function getGlobalService(): Service;
export function createService(name: string): Service;
export function createServiceWithConfig(name: string, config: ServiceConfig): Service;
export interface Service {
  /** Placeholder; full types from generated bindings. */
  [key: string]: unknown;
}
export interface Session {
  [key: string]: unknown;
}
export interface MessageContext {
  [key: string]: unknown;
}
export interface ReceivedMessage {
  context: MessageContext;
  payload: ArrayBuffer;
}
export type BuildInfo = { version: string; gitSha: string; buildDate: string; profile: string };
export function getBuildInfo(): BuildInfo;
