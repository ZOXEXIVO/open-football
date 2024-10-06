import {Component, OnInit} from '@angular/core';
import {ActivatedRoute} from '@angular/router';
import {UntilDestroy} from '@ngneat/until-destroy';
import {MatchDto, MatchService} from "../services/match.service";
import {MatchPlayService} from "../services/match.play.service";
import {MatchDataService} from "../services/match.data.service";
import {TitleService} from "../../shared/services/title.service";
import {TopHeaderService} from "../../shared/top-header/services/top.header.service";

@UntilDestroy()
@Component({
    templateUrl: './match.get.component.html',
    styleUrls: ['./match.get.component.scss']
})
export class MatchGetComponent implements OnInit {
    public match: MatchDto | null = null;

    leagueSlug: string = '';
    matchId: string = '';

    matchTimeMs: number = 0;

    constructor(private matchPlayService: MatchPlayService,
                public matchDataService: MatchDataService,
                private matchService: MatchService,
                private titleService: TitleService,
                private topHeaderService: TopHeaderService,
                private route: ActivatedRoute) {

        this.leagueSlug = this.route.snapshot.params["league_slug"];
        this.matchId = this.route.snapshot.params["match_id"];
    }

    ngOnInit(): void {
        this.matchService.get(this.leagueSlug, this.matchId).subscribe(data => {
            this.match = data;
            this.matchDataService.setData(data);

            this.titleService.setTitle(`${data?.home_team_name} : ${data?.away_team_name}`)
            this.topHeaderService.setContent(`${data?.home_team_name} ${data?.score.home_goals} : ${data?.score.away_goals} ${data?.away_team_name}`, '', '/', false);
        });
    }
}
