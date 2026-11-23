/** True only for an https link on github.com itself (the one place update links may point). */
export function isGithubUrl(url: string | null | undefined): url is string {
  return typeof url === "string" && url.startsWith("https://github.com/");
}
