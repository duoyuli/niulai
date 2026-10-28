set -e
python3 - <<'EOF'
import re
p = 'backend/config/datasets.yaml'
t = open(p).read()
t = re.sub(r'  - id: fund\.qdii_quota\n(?:    .*\n)+\n', '', t)
t = t.replace('（fund.qdii_quota 的分类汇总，需要时再加）', '（QDII 额度分类汇总，需要时再加）')
open(p, 'w').write(t)
p = 'docs/interfaces/dataset_catalog.md'
t = open(p).read()
t = t.replace('纳入 85 个数据集', '纳入 84 个数据集').replace('85 个数据集中，P0 9 个、P1 20 个、P2 20 个、P3 36 个', '84 个数据集中，P0 9 个、P1 20 个、P2 20 个、P3 35 个').replace('、QDII 额度；', '；')
open(p, 'w').write(t)
p = 'docs/interfaces/index.md'
t = open(p).read().replace('85 个数据集', '84 个数据集')
open(p, 'w').write(t)
EOF
grep -q 'qdii_quota' backend/config/datasets.yaml && exit 1 || true
