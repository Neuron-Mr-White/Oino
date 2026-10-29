import { z } from "zod";

/**
 * Port forwarding wire types (UNI-213, forwarding phase 1). Mirrors
 * `crates/proto/src/forward.rs`: `ports_list` (request/reply) and
 * `tcp_forward{port}` (hello, `ok{}`, then raw bytes both ways — like
 * `ssh -L` to the host's loopback).
 */

export const ForwardPortSchema = z
  .number()
  .int("port must be an integer")
  .min(1, "port must be 1..65535")
  .max(65535, "port must be 1..65535");

export const ListeningPortSchema = z.object({
  port: z.number().int().min(1).max(65535),
  bind: z.string(),
  pid: z.number().int().min(0).optional(),
  process: z.string().optional(),
  cmd: z.string().optional(),
  http: z.boolean().default(false),
  allowed: z.boolean().default(false),
});
export type ListeningPort = z.infer<typeof ListeningPortSchema>;

export const PortsListDataSchema = z.object({
  ports: z.array(ListeningPortSchema),
  any: z.boolean(),
  allowed: z.array(z.number().int().min(1).max(65535)).default([]),
});
export type PortsListData = z.infer<typeof PortsListDataSchema>;
