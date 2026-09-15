export function BrandMark({
  size = "sm",
  wordmark = "qMonitor",
}: {
  size?: "sm" | "md";
  wordmark?: string;
}) {
  const px = size === "md" ? 36 : 28;
  return (
    <div className={`brand-mark brand-mark--${size}`}>
      <img src="/favicon.svg" alt="" width={px} height={px} />
      <span>{wordmark}</span>
    </div>
  );
}
