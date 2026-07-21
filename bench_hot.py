import subprocess, time

proc = subprocess.Popen(
    ["target/release/cheime-engine.exe", "--dict-dir", "data/dicts", "--stdin"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    text=True, bufsize=1
)
# Drain startup
for _ in range(6):
    proc.stderr.readline()

# Run 10 warmup rounds
for _ in range(10):
    for c in "nihao":
        proc.stdin.write(c + "\n"); proc.stdin.flush()
        proc.stdout.readline(); proc.stdout.readline()
    proc.stdin.write("\x08\n"); proc.stdin.flush()  # backspace
    for _ in range(10):
        proc.stdout.readline()

codes = ["n", "ni", "nih", "nihao", "zho", "zhon", "zhong", "xian", "shu", "shua", "shuan"]
for code in codes:
    # Reset: backspace until empty
    for _ in range(10):
        proc.stdin.write("\x08\n"); proc.stdin.flush()
        for _ in range(2):
            proc.stdout.readline()

    # Type code, measure last key
    for i, c in enumerate(code):
        proc.stdin.write(c + "\n"); proc.stdin.flush()
        proc.stdout.readline()
        if i == len(code) - 1:
            t0 = time.perf_counter()
        result = proc.stdout.readline()
        if i == len(code) - 1:
            t1 = time.perf_counter()

    ncand = result.count('"text"')
    us = (t1 - t0) * 1_000_000
    print(f"  {code:8s} → {us:8.0f} µs  ({ncand} cand)")

proc.stdin.write("quit\n"); proc.stdin.flush()
proc.wait()
