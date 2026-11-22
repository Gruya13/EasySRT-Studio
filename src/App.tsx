import { Subtitles } from "lucide-react";

export function App() {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center bg-neutral-950 p-6 text-neutral-100">
      <div className="flex max-w-md flex-col items-center gap-4 rounded-2xl border border-neutral-800 bg-neutral-900/70 p-8 text-center shadow-lg">
        <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-indigo-500/15 text-indigo-400">
          <Subtitles className="h-6 w-6" />
        </div>
        <h1 className="text-2xl font-semibold tracking-tight">
          EasySRT Studio
        </h1>
        <p className="text-sm text-neutral-400">
          Offline-first transcription, segmentation, and Serbian subtitle
          translation workspace.
        </p>
      </div>
    </main>
  );
}

export default App;
