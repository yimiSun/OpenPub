export interface Article {
  id: string;
  title: string;
  content: string;
  createdAt: string;
  updatedAt: string;
}

export function articleLabel(article: Pick<Article, "title">): string {
  const trimmed = article.title.trim();
  return trimmed.length > 0 ? trimmed : "未命名文章";
}