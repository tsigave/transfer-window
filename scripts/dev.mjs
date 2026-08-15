import { spawn } from 'node:child_process'

const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm'
const processes = [
  spawn('cargo', ['run', '-p', 'sim-server'], { stdio: 'inherit' }),
  spawn(npm, ['run', 'dev:web'], { stdio: 'inherit' }),
]

let stopping = false
function stop(signal = 'SIGTERM') {
  if (stopping) return
  stopping = true
  for (const child of processes) {
    if (!child.killed) child.kill(signal)
  }
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => stop(signal))
}

for (const child of processes) {
  child.on('error', (error) => {
    console.error(error)
    process.exitCode = 1
    stop()
  })
  child.on('exit', (code, signal) => {
    if (!stopping) {
      process.exitCode = code ?? (signal ? 1 : 0)
      stop()
    }
  })
}
