import { useTheme } from "next-themes";

export function ThemeToggle() {
  const { setTheme, resolvedTheme } = useTheme();

  const toggleTheme = () => {
    setTheme(resolvedTheme === "dark" ? "light" : "dark");
  };

  return (
    <button
      type="button"
      onClick={toggleTheme}
      className="w-4 h-4 rounded-full! bg-foreground transition-colors duration-300 hover:opacity-80"
      aria-label="Toggle theme"
    />
  );
}
