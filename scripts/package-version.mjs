// Keep the complete release version in filenames, build-info and display text.
// OS metadata has narrower syntax: NSIS requires four integers, Apple requires
// numeric bundle versions, and Debian uses "~" to sort a preview before stable.
export function packageVersions(version) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(version);
  if (!match || match[0] !== version || match.slice(1, 4).some(n => Number(n) > 65535) ||
      match[4]?.split('.').some(part => /^\d+$/.test(part) && part.length > 1 && part.startsWith('0'))) {
    throw Error(`Invalid or unsupported package version: ${version}`);
  }
  const core = match.slice(1, 4).join('.');
  const prerelease = match[4] ?? '';
  // Avoid treating a SemVer prerelease hyphen as Debian's revision separator.
  const debian = core + (prerelease ? `~${prerelease.toLowerCase().replaceAll('-', '.')}` : '') +
    (match[5] ? `+${match[5].replaceAll('-', '.')}` : '');
  return { display: version, core, prerelease, windows: `${core}.0`, macos: core, debian };
}
