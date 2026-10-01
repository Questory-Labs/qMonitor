import { invoke } from "@tauri-apps/api/core";

export async function invokeTimeout<T>(
  cmd: string,
  ms = 4000,
  args?: Record<string, unknown>,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      invoke<T>(cmd, args),
      new Promise<T>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${cmd} timed out`)), ms);
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}
