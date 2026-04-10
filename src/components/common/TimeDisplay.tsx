// Displays a duration in MM:SS.ms format.

interface Props {
  ms: number;
  className?: string;
}

export function TimeDisplay({ ms, className }: Props) {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  const tenths = Math.floor((ms % 1000) / 100);

  return (
    <span className={className}>
      {String(minutes).padStart(2, "0")}:{String(seconds).padStart(2, "0")}.
      {tenths}
    </span>
  );
}
