# Isolated collection for the write workloads, so the 50-row read dataset
# stays pristine. Seeded with 800,000 documents keyed "1".."800000".
class Wpost < Model
end
