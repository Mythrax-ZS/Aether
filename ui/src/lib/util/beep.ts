// The terminal bell as a short tone. One audio context for the whole app: a browser allows only a handful at a time, and a
// context per beep (or per pane) runs out quickly when a program rings the bell in a loop.

let context: AudioContext | undefined;

export function beep(): void {
  try {
    context ??= new AudioContext();
    if (context.state === 'suspended') void context.resume();
    const osc = context.createOscillator();
    const gain = context.createGain();
    osc.frequency.value = 880;
    gain.gain.value = 0.04;
    osc.connect(gain).connect(context.destination);
    osc.start();
    osc.stop(context.currentTime + 0.09);
  } catch {
    /* no audio device */
  }
}
