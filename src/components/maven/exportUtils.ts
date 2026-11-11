/**
 * Maven 依赖清单多格式导出工具：
 * 支持导出为 Markdown 表格、CSV 合规审计表、Gradle build.gradle 依赖块、纯 GAV 清单文本。
 */
import { checkCve } from '@/components/maven/cveDatabase';
import type { MavenTreeNode } from '@/services/mavenService';

/** 过滤出最终采纳的有效构件（Winner 节点），按 GA 唯一去重 */
export function getUniqueWinners(nodes: MavenTreeNode[]): MavenTreeNode[] {
  const seen = new Set<string>();
  const winners: MavenTreeNode[] = [];
  for (const n of nodes) {
    if (n.loser) continue;
    const key = `${n.group}:${n.artifact}:${n.version}`;
    if (!seen.has(key)) {
      seen.add(key);
      winners.push(n);
    }
  }
  return winners;
}

/** 1. 导出为 Markdown 表格（适合上线物料、文档归档、三方合规声明） */
export function exportToMarkdown(nodes: MavenTreeNode[]): string {
  const winners = getUniqueWinners(nodes);
  const rows: string[] = [
    '# Maven 依赖清单报告',
    '',
    `> 共解析有效依赖构件 ${winners.length} 项（已调解并排除冲突冗余版本）。`,
    '',
    '| 序号 | 组织 (GroupId) | 构件 (ArtifactId) | 版本 (Version) | 打包 | Scope | 安全风险 (CVE) |',
    '| :---: | :--- | :--- | :--- | :---: | :---: | :--- |',
  ];

  winners.forEach((n, idx) => {
    const cve = checkCve(n.group, n.artifact, n.version);
    const cveText = cve ? `⚠️ **[${cve.severity}]** ${cve.cve} (${cve.recommendedVersion})` : '正常 (安全)';
    rows.push(
      `| ${idx + 1} | \`${n.group}\` | \`${n.artifact}\` | \`${n.version}\` | \`${n.packaging}\` | \`${n.scope}\` | ${cveText} |`,
    );
  });

  return rows.join('\n');
}

/** 2. 导出为 CSV 格式（带 UTF-8 BOM，Excel 双击不乱码） */
export function exportToCsv(nodes: MavenTreeNode[]): string {
  const winners = getUniqueWinners(nodes);
  const rows: string[] = [
    '\uFEFF"序号","GroupId","ArtifactId","Version","Packaging","Scope","Status","CVE_Alert"',
  ];

  winners.forEach((n, idx) => {
    const cve = checkCve(n.group, n.artifact, n.version);
    const cveText = cve ? `${cve.cve} (${cve.severity})` : '';
    rows.push(
      `"${idx + 1}","${n.group}","${n.artifact}","${n.version}","${n.packaging}","${n.scope}","Winner","${cveText}"`,
    );
  });

  return rows.join('\r\n');
}

/** 3. 导出为 Gradle build.gradle 依赖块 */
export function exportToGradle(nodes: MavenTreeNode[]): string {
  const winners = getUniqueWinners(nodes);
  const lines: string[] = ['dependencies {'];

  for (const n of winners) {
    let conf = 'implementation';
    switch (n.scope.toLowerCase()) {
      case 'test':
        conf = 'testImplementation';
        break;
      case 'provided':
        conf = 'compileOnly';
        break;
      case 'runtime':
        conf = 'runtimeOnly';
        break;
      default:
        conf = 'implementation';
        break;
    }
    const cls = n.classifier ? `:${n.classifier}` : '';
    lines.push(`    ${conf} '${n.group}:${n.artifact}:${n.version}${cls}'`);
  }

  lines.push('}');
  return lines.join('\n');
}

/** 4. 导出为纯 GAV 清单（每行一个坐标） */
export function exportToGavList(nodes: MavenTreeNode[]): string {
  const winners = getUniqueWinners(nodes);
  return winners.map((n) => `${n.group}:${n.artifact}:${n.version}`).join('\n');
}
