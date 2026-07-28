import { describe, expect, it } from "vitest";
import { validateDeploymentOrigin } from "./deployment-url";

describe("deployment origin validation", () => {
  it.each([
    "http://localhost:3000",
    "http://127.0.0.1:8080",
    "http://[::1]:8787"
  ])("allows development loopback origin %s", (origin) => {
    expect(validateDeploymentOrigin(origin, false)).toBe(origin);
  });

  it("requires HTTPS for loopback in production", () => {
    expect(() => validateDeploymentOrigin("http://localhost:3000", true)).toThrow("HTTPS");
  });

  it("rejects non-loopback plain HTTP and URL paths", () => {
    expect(() => validateDeploymentOrigin("http://192.168.1.10:8080", false)).toThrow("HTTPS");
    expect(() => validateDeploymentOrigin("https://agent.example/api", false)).toThrow(
      "deployment origin"
    );
  });
});
