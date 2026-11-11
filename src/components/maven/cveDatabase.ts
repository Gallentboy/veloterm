/**
 * 本地离线高危 Java 构件 CVE 漏洞特征库。
 * 针对 Java 开发者常见的高危 RCE（远程代码执行）、反序列化漏洞进行离线规则匹配与预警。
 */

export interface CveRule {
  cve: string;
  name: string;
  severity: 'CRITICAL' | 'HIGH';
  group: string;
  artifacts: string[];
  description: string;
  recommendedVersion: string;
  isVulnerable: (version: string) => boolean;
}

/** 拆分并标准化版本段供比较 */
function parseVersionSegments(v: string): Array<number | string> {
  const clean = v.trim().replace(/^v/i, '').split('-')[0] ?? '';
  return clean.split('.').map((part) => {
    const num = parseInt(part, 10);
    return isNaN(num) ? part.toLowerCase() : num;
  });
}

/** 简易版本大小比较：v1 < v2 返回 -1，v1 === v2 返回 0，v1 > v2 返回 1 */
export function compareVersion(v1: string, v2: string): number {
  const segs1 = parseVersionSegments(v1);
  const segs2 = parseVersionSegments(v2);
  const len = Math.max(segs1.length, segs2.length);
  for (let i = 0; i < len; i++) {
    const s1 = segs1[i] ?? 0;
    const s2 = segs2[i] ?? 0;
    if (typeof s1 === 'number' && typeof s2 === 'number') {
      if (s1 < s2) return -1;
      if (s1 > s2) return 1;
    } else {
      const str1 = String(s1);
      const str2 = String(s2);
      if (str1 < str2) return -1;
      if (str1 > str2) return 1;
    }
  }
  return 0;
}

/** 检查版本是否在 [min, max] 区间（闭区间或包含） */
function inRange(v: string, min: string, max: string): boolean {
  return compareVersion(v, min) >= 0 && compareVersion(v, max) <= 0;
}

export const KNOWN_CVE_RULES: CveRule[] = [
  {
    cve: 'CVE-2021-44228',
    name: 'Log4j2 JNDI 远程代码执行漏洞 (Log4Shell)',
    severity: 'CRITICAL',
    group: 'org.apache.logging.log4j',
    artifacts: ['log4j-core'],
    description: 'JNDI 注入漏洞允许未经身份验证的远程攻击者在服务端执行任意代码。',
    recommendedVersion: '>= 2.17.1',
    isVulnerable: (v) => inRange(v, '2.0', '2.14.1') || v.startsWith('2.15.0'),
  },
  {
    cve: 'CVE-2022-22965',
    name: 'Spring Framework 远程代码执行漏洞 (Spring4Shell)',
    severity: 'CRITICAL',
    group: 'org.springframework',
    artifacts: ['spring-beans', 'spring-webmvc', 'spring-core'],
    description: '通过 DataBinder 参数绑定可写入恶意文件实现远程命令执行（需 JDK 9+ 与 Tomcat 部署）。',
    recommendedVersion: '>= 5.3.18 / 5.2.20',
    isVulnerable: (v) => inRange(v, '5.3.0', '5.3.17') || inRange(v, '5.2.0', '5.2.19'),
  },
  {
    cve: 'CVE-2022-22963',
    name: 'Spring Cloud Function SpEL 表达式注入远程命令执行',
    severity: 'CRITICAL',
    group: 'org.springframework.cloud',
    artifacts: ['spring-cloud-function-context'],
    description: 'spring.cloud.function.routing-expression HTTP 头未安全过滤导致 SpEL 代码执行。',
    recommendedVersion: '>= 3.2.3 / 3.1.7',
    isVulnerable: (v) => inRange(v, '3.0.0', '3.2.2'),
  },
  {
    cve: 'CVE-2022-25845',
    name: 'Fastjson 历史全系反序列化远程代码执行 (AutoType Bypass)',
    severity: 'CRITICAL',
    group: 'com.alibaba',
    artifacts: ['fastjson'],
    description: 'AutoType 机制绕过漏洞，构造特定反序列化链导致远程代码执行。',
    recommendedVersion: '>= 1.2.83 或升级至 fastjson2',
    isVulnerable: (v) => compareVersion(v, '1.2.83') < 0,
  },
  {
    cve: 'CVE-2022-1471',
    name: 'SnakeYAML Constructor 反序列化远程代码执行',
    severity: 'HIGH',
    group: 'org.yaml',
    artifacts: ['snakeyaml'],
    description: 'Constructor 默认支持任意类型实例化，解析不可信 yaml 导致远程代码执行。',
    recommendedVersion: '>= 2.0',
    isVulnerable: (v) => compareVersion(v, '2.0') < 0,
  },
  {
    cve: 'CVE-2021-39144',
    name: 'XStream 默认反序列化代码执行漏洞',
    severity: 'CRITICAL',
    group: 'com.thoughtworks.xstream',
    artifacts: ['xstream'],
    description: '未配置安全黑白名单的默认 XStream 实例在反序列化特制 XML 时导致任意代码执行。',
    recommendedVersion: '>= 1.4.19',
    isVulnerable: (v) => compareVersion(v, '1.4.19') < 0,
  },
  {
    cve: 'CVE-2020-36518',
    name: 'Jackson-databind 深入嵌套深度拒绝服务与多态反序列化',
    severity: 'HIGH',
    group: 'com.fasterxml.jackson.core',
    artifacts: ['jackson-databind'],
    description: '特制反序列化 JSON 触发 StackOverflowError 拒绝服务或多态类型注入。',
    recommendedVersion: '>= 2.13.2.1 / 2.12.6.1',
    isVulnerable: (v) => inRange(v, '2.10.0', '2.13.2') && v !== '2.13.2.1',
  },
  {
    cve: 'CVE-2020-1957',
    name: 'Apache Shiro 权限绕过漏洞',
    severity: 'HIGH',
    group: 'org.apache.shiro',
    artifacts: ['shiro-core', 'shiro-web'],
    description: 'URL 规范化差异导致 Spring 与 Shiro 路径映射不一致，造成未授权越权访问。',
    recommendedVersion: '>= 1.5.2',
    isVulnerable: (v) => compareVersion(v, '1.5.2') < 0,
  },
  {
    cve: 'CVE-2015-7501',
    name: 'Apache Commons Collections 经典 Java 反序列化 Gadget',
    severity: 'CRITICAL',
    group: 'commons-collections',
    artifacts: ['commons-collections'],
    description: 'InvokerTransformer 反射链在受信任输入之外反序列化时可执行任意 Java 代码。',
    recommendedVersion: '>= 3.2.2 或 commons-collections4',
    isVulnerable: (v) => compareVersion(v, '3.2.2') < 0,
  },
];

/** 匹配单个构件是否存在已知高危 CVE */
export function checkCve(group: string, artifact: string, version: string): CveRule | null {
  for (const rule of KNOWN_CVE_RULES) {
    if (rule.group === group && rule.artifacts.includes(artifact)) {
      if (rule.isVulnerable(version)) {
        return rule;
      }
    }
  }
  return null;
}
