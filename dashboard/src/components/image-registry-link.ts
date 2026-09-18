// Docker Distribution reference syntax: github.com/distribution/reference/blob/main/regexp.go
const pathComponent = "[a-z0-9]+(?:(?:[._]|__|-+)[a-z0-9]+)*";
const domainComponent = "(?:[a-zA-Z0-9]|[a-zA-Z0-9][a-zA-Z0-9-]*[a-zA-Z0-9])";
const registry = `(?:${domainComponent}(?:\\.${domainComponent})*|\\[[a-fA-F0-9:]+\\])(?::[0-9]+)?`;
const imageReferencePattern = new RegExp(
  `^(?:${registry}/)?${pathComponent}(?:/${pathComponent})*(?::[\\w][\\w.-]{0,127})?(?:@[A-Za-z][A-Za-z0-9]*(?:[-_+.][A-Za-z][A-Za-z0-9]*)*:[a-fA-F0-9]{32,})?$`,
);

export function isValidImageReference(image: string): boolean {
  return image.length <= 500 && imageReferencePattern.test(image);
}

export function imageRegistryLink(image: string): string | null {
  if (!isValidImageReference(image.trim())) return null;
  const repository = image.trim().split("@")[0]?.replace(/:[^/]+$/, "") ?? "";

  const parts = repository.split("/");
  const first = parts[0]!;
  const hasRegistry = parts.length > 1 && (/[.:]/.test(first) || first === "localhost");
  const registry = hasRegistry ? parts.shift()! : "docker.io";
  if (parts.some((part) => !part || part === "." || part === "..")) return null;
  const path = parts.map(encodeURIComponent).join("/");

  if (["docker.io", "index.docker.io", "registry-1.docker.io"].includes(registry)) {
    return parts.length === 1 || parts[0] === "library"
      ? `https://hub.docker.com/_/${parts.at(-1)}`
      : `https://hub.docker.com/r/${path}`;
  }
  if (registry === "quay.io") return `https://quay.io/repository/${path}`;
  return `https://${registry}/${path}`;
}
