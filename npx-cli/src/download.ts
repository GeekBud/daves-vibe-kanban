import https from 'https';
import fs from 'fs';
import path from 'path';
import crypto from 'crypto';
import os from 'os';

// Replaced during npm pack by workflow
export const R2_BASE_URL = '__R2_PUBLIC_URL__';
export const BINARY_TAG = '__BINARY_TAG__'; // e.g., v0.0.135-20251215122030
export const CACHE_DIR = path.join(os.homedir(), '.vibe-kanban', 'bin');

// GitHub Releases fallback for when R2 CDN is unavailable
const GITHUB_REPO = 'GeekBud/daves-vibe-kanban';
const GITHUB_RELEASES_BASE_URL = `https://github.com/${GITHUB_REPO}/releases/download`;
const PACKAGE_VERSION = require('../package.json').version as string;

function isPlaceholder(value: string): boolean {
  return value.startsWith('__') && value.endsWith('__');
}

export function effectiveTag(): string {
  return isPlaceholder(BINARY_TAG) ? `v${PACKAGE_VERSION}` : BINARY_TAG;
}

function r2UrlAvailable(): boolean {
  return !isPlaceholder(R2_BASE_URL);
}

// Local development mode: use binaries from npx-cli/dist/ instead of R2
// Only activate if dist/ exists (i.e., running from source after local-build.sh)
export const LOCAL_DIST_DIR = path.join(__dirname, '..', 'dist');
export const LOCAL_DEV_MODE =
  fs.existsSync(LOCAL_DIST_DIR) ||
  process.env.VIBE_KANBAN_LOCAL === '1';

export interface BinaryInfo {
  sha256: string;
  size: number;
}

export interface BinaryManifest {
  latest?: string;
  platforms: Record<string, Record<string, BinaryInfo>>;
}

export interface DesktopPlatformInfo {
  file: string;
  sha256: string;
  type: string | null;
}

export interface DesktopManifest {
  platforms: Record<string, DesktopPlatformInfo>;
}

export interface DesktopBundleInfo {
  archivePath: string | null;
  dir: string;
  type: string | null;
}

type ProgressCallback = (downloaded: number, total: number) => void;

function fetchJson<T>(url: string): Promise<T> {
  return new Promise((resolve, reject) => {
    https
      .get(url, (res) => {
        if (res.statusCode === 301 || res.statusCode === 302) {
          return fetchJson<T>(res.headers.location!)
            .then(resolve)
            .catch(reject);
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode} fetching ${url}`));
        }
        let data = '';
        res.on('data', (chunk: string) => (data += chunk));
        res.on('end', () => {
          try {
            resolve(JSON.parse(data) as T);
          } catch {
            reject(new Error(`Failed to parse JSON from ${url}`));
          }
        });
      })
      .on('error', reject);
  });
}

function downloadFile(
  url: string,
  destPath: string,
  expectedSha256: string | undefined,
  onProgress?: ProgressCallback
): Promise<string> {
  const tempPath = destPath + '.tmp';
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(tempPath);
    const hash = crypto.createHash('sha256');

    const cleanup = () => {
      try {
        fs.unlinkSync(tempPath);
      } catch {}
    };

    https
      .get(url, (res) => {
        if (res.statusCode === 301 || res.statusCode === 302) {
          file.close();
          cleanup();
          return downloadFile(
            res.headers.location!,
            destPath,
            expectedSha256,
            onProgress
          )
            .then(resolve)
            .catch(reject);
        }

        if (res.statusCode !== 200) {
          file.close();
          cleanup();
          return reject(
            new Error(`HTTP ${res.statusCode} downloading ${url}`)
          );
        }

        const totalSize = parseInt(
          res.headers['content-length'] || '0',
          10
        );
        let downloadedSize = 0;

        res.on('data', (chunk: Buffer) => {
          downloadedSize += chunk.length;
          hash.update(chunk);
          if (onProgress) onProgress(downloadedSize, totalSize);
        });
        res.pipe(file);

        file.on('finish', () => {
          file.close();
          const actualSha256 = hash.digest('hex');
          if (expectedSha256 && actualSha256 !== expectedSha256) {
            cleanup();
            reject(
              new Error(
                `Checksum mismatch: expected ${expectedSha256}, got ${actualSha256}`
              )
            );
          } else {
            try {
              fs.renameSync(tempPath, destPath);
              resolve(destPath);
            } catch (err) {
              cleanup();
              reject(err);
            }
          }
        });
      })
      .on('error', (err) => {
        file.close();
        cleanup();
        reject(err);
      });
  });
}

export async function ensureBinary(
  platform: string,
  binaryName: string,
  onProgress?: ProgressCallback
): Promise<string> {
  // In local dev mode, use binaries directly from npx-cli/dist/
  if (LOCAL_DEV_MODE) {
    const localZipPath = path.join(
      LOCAL_DIST_DIR,
      platform,
      `${binaryName}.zip`
    );
    if (fs.existsSync(localZipPath)) {
      return localZipPath;
    }
    throw new Error(
      `Local binary not found: ${localZipPath}\n` +
        `Run ./local-build.sh first to build the binaries, or set VIBE_KANBAN_LOCAL=1.`
    );
  }

  const tag = effectiveTag();
  const cacheDir = path.join(CACHE_DIR, tag, platform);
  const zipPath = path.join(cacheDir, `${binaryName}.zip`);

  if (fs.existsSync(zipPath)) return zipPath;

  fs.mkdirSync(cacheDir, { recursive: true });

  let binaryInfo: BinaryInfo | undefined;
  let url: string | undefined;

  if (r2UrlAvailable()) {
    try {
      const manifest = await fetchJson<BinaryManifest>(
        `${R2_BASE_URL}/binaries/${BINARY_TAG}/manifest.json`
      );
      binaryInfo = manifest.platforms?.[platform]?.[binaryName];
      if (binaryInfo) {
        url = `${R2_BASE_URL}/binaries/${BINARY_TAG}/${platform}/${binaryName}.zip`;
      }
    } catch {
      // R2 failed, will try fallback
    }
  }

  if (!url) {
    // Fallback to GitHub Releases
    url = `${GITHUB_RELEASES_BASE_URL}/${tag}/${binaryName}-${platform}.zip`;
    // GitHub Releases may not have a manifest with SHA256, so we skip checksum validation
    binaryInfo = undefined;
  }

  if (!url) {
    throw new Error(
      `Binary ${binaryName} not available for ${platform}. ` +
      `No R2 CDN configured and no GitHub Release fallback available. ` +
      `Try setting VIBE_KANBAN_LOCAL=1 and building from source.`
    );
  }

  try {
    await downloadFile(url, zipPath, binaryInfo?.sha256, onProgress);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    throw new Error(
      `Failed to download ${binaryName} for ${platform}: ${msg}\n` +
      `If you are building from source, run ./local-build.sh and set VIBE_KANBAN_LOCAL=1.`
    );
  }

  return zipPath;
}

export const DESKTOP_CACHE_DIR = path.join(
  os.homedir(),
  '.vibe-kanban',
  'desktop'
);

export async function ensureDesktopBundle(
  tauriPlatform: string,
  onProgress?: ProgressCallback
): Promise<DesktopBundleInfo> {
  // In local dev mode, use Tauri bundle from npx-cli/dist/tauri/<platform>/
  if (LOCAL_DEV_MODE) {
    const localDir = path.join(LOCAL_DIST_DIR, 'tauri', tauriPlatform);
    if (fs.existsSync(localDir)) {
      const files = fs.readdirSync(localDir);
      const archive = files.find(
        (f) => f.endsWith('.tar.gz') || f.endsWith('-setup.exe')
      );
      return {
        dir: localDir,
        archivePath: archive ? path.join(localDir, archive) : null,
        type: null,
      };
    }
    throw new Error(
      `Local desktop bundle not found: ${localDir}\n` +
        `Run './local-build.sh --desktop' first to build the Tauri app, or set VIBE_KANBAN_LOCAL=1.`
    );
  }

  const tag = effectiveTag();
  const cacheDir = path.join(
    DESKTOP_CACHE_DIR,
    tag,
    tauriPlatform
  );

  // Check if already installed (sentinel file from previous run)
  const sentinelPath = path.join(cacheDir, '.installed');
  if (fs.existsSync(sentinelPath)) {
    return { dir: cacheDir, archivePath: null, type: null };
  }

  fs.mkdirSync(cacheDir, { recursive: true });

  let platformInfo: DesktopPlatformInfo | undefined;
  let url: string | undefined;

  if (r2UrlAvailable()) {
    try {
      const manifest = await fetchJson<DesktopManifest>(
        `${R2_BASE_URL}/binaries/${BINARY_TAG}/tauri/desktop-manifest.json`
      );
      platformInfo = manifest.platforms?.[tauriPlatform];
      if (platformInfo) {
        url = `${R2_BASE_URL}/binaries/${BINARY_TAG}/tauri/${tauriPlatform}/${platformInfo.file}`;
      }
    } catch {
      // R2 failed, will try fallback
    }
  }

  if (!url) {
    // Fallback: try common GitHub Release naming conventions for Tauri bundles
    const candidates = [
      `vibe-kanban_${tag}_${tauriPlatform}.tar.gz`,
      `vibe-kanban_${tag}_${tauriPlatform}.dmg`,
      `vibe-kanban_${tag}_${tauriPlatform}-setup.exe`,
      `vibe-kanban_${tauriPlatform}.tar.gz`,
      `vibe-kanban_${tauriPlatform}.dmg`,
      `vibe-kanban_${tauriPlatform}-setup.exe`,
    ];
    for (const file of candidates) {
      const testUrl = `${GITHUB_RELEASES_BASE_URL}/${tag}/${file}`;
      try {
        // HEAD request to check existence
        const exists = await new Promise<boolean>((resolve) => {
          https.request(testUrl, { method: 'HEAD' }, (res) => {
            resolve(res.statusCode === 200);
          }).on('error', () => resolve(false)).end();
        });
        if (exists) {
          url = testUrl;
          platformInfo = { file, sha256: '', type: null };
          break;
        }
      } catch {
        // continue to next candidate
      }
    }
  }

  if (!url || !platformInfo) {
    throw new Error(
      `Desktop app not available for platform: ${tauriPlatform}. ` +
      `No R2 CDN configured and no GitHub Release fallback found. ` +
      `Try setting VIBE_KANBAN_LOCAL=1 and building from source.`
    );
  }

  const destPath = path.join(cacheDir, platformInfo.file);

  // Skip download if file already exists (e.g. previous failed install)
  if (!fs.existsSync(destPath)) {
    try {
      await downloadFile(url, destPath, platformInfo.sha256 || undefined, onProgress);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      throw new Error(
        `Failed to download desktop bundle for ${tauriPlatform}: ${msg}\n` +
        `If you are building from source, run './local-build.sh --desktop' and set VIBE_KANBAN_LOCAL=1.`
      );
    }
  }

  return {
    archivePath: destPath,
    dir: cacheDir,
    type: platformInfo.type,
  };
}

export async function getLatestVersion(): Promise<string | undefined> {
  if (r2UrlAvailable()) {
    try {
      const manifest = await fetchJson<BinaryManifest>(
        `${R2_BASE_URL}/binaries/manifest.json`
      );
      if (manifest.latest) {
        return manifest.latest;
      }
    } catch {
      // R2 failed, try fallback
    }
  }

  // Fallback: use package.json version
  return `v${PACKAGE_VERSION}`;
}
