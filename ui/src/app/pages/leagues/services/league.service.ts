import { HttpClient } from "@angular/common/http";
import { Injectable } from "@angular/core";
import { Observable } from "rxjs";

@Injectable({
   providedIn: 'root',
})
export class LeagueService {
   constructor(private http: HttpClient) {
   }

   get(slug: String): Observable<LeagueDto> {
      return this.http.get<LeagueDto>('/api/leagues/' + slug);
   }
}

export interface LeagueDto {
   name: String,
   slug: String,
   country_slug: String,
   country_name: String,
   table: LeagueTableDto,
   current_tour_schedule: LeagueTourScheduleDto[]
}

export interface LeagueTableDto {
   rows: LeagueTableItemDto[]
}

export interface LeagueTableItemDto {
   team_id: number,
   team_name: String,
   team_slug: String,
   played: number,
   win: number,
   draft: number,
   lost: number,
   goal_scored: number,
   goal_concerned: number,
   points: number
}

export interface LeagueTourScheduleDto {
   date: String,
   matches: LeagueTourScheduleMatchDto[],
}

export interface LeagueTourScheduleMatchDto {
   match_id: String,
   home_team_id: number,
   home_team_name: String,
   home_team_slug: String,
   away_team_id: number,
   away_team_name: String,
   away_team_slug: String,
   result: LeagueTourScheduleMatchResultDto
}

export interface LeagueTourScheduleMatchResultDto {
   home_goals: number,
   home_goalscorers: LeagueTableGoalscorerDto[]
   away_goals: number,
   away_goalscorers: LeagueTableGoalscorerDto[]
}

export interface LeagueTableGoalscorerDto {
   id: number,
   name: number
}